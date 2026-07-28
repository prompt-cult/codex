/// The runner: instantiates a compiled ThreadBox guest module under
/// `wasmi`, captures the single `emit(ptr, len)` call, and hands the
/// captured bytes to `threadbox-ir::parse` + `validate`. See
/// `README.md` — the `tb-run` row — and `AGENTS.md` — "wasmi only in
/// the runner" — for why this crate alone carries a dependency beyond
/// `threadbox-ir`.
use std::path::Path;
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = match args.as_slice() {
        [_, p] => p.clone(),
        _ => {
            eprintln!("usage: tb-run <path-to-asc-source-or-wasm>");
            std::process::exit(1);
        }
    };

    let wasm_bytes = if path.ends_with(".ts") {
        match compile_ts_to_wasm(&path) {
            Ok(bytes) => bytes,
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
    } else {
        std::fs::read(&path).unwrap_or_else(|e| {
            eprintln!("cannot read \"{path}\": {e}");
            std::process::exit(1);
        })
    };

    let json = match run_wasm(&wasm_bytes) {
        Ok(json) => json,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    match threadbox_ir::parse(&json).and_then(|graph| threadbox_ir::validate(&graph).map(|_| graph)) {
        Ok(_) => {
            println!("{json}");
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

/// Compile a guest `.ts` source to wasm bytes by shelling out to `asc`.
/// Runs from the `guest/` directory that is the source's ancestor, so
/// `asc` finds `node_modules/assemblyscript`. `AGENTS.md`'s "no
/// framework" rule does not reach across this process boundary: `asc`
/// is a build tool invoked as a subprocess, not a Rust dependency.
fn compile_ts_to_wasm(source_path: &str) -> Result<Vec<u8>, String> {
    let source = Path::new(source_path);
    let guest_dir = find_guest_dir(source)
        .ok_or_else(|| format!("\"{source_path}\" is not inside a guest/ directory; cannot locate node_modules/assemblyscript"))?;

    let relative_source = source
        .strip_prefix(&guest_dir)
        .map_err(|_| format!("\"{source_path}\" is not inside \"{}\"", guest_dir.display()))?;

    let out_path = std::env::temp_dir().join(format!(
        "tb-run-{}.wasm",
        std::process::id()
    ));

    let output = Command::new("npx")
        .args(["asc", &relative_source.to_string_lossy(), "-o", &out_path.to_string_lossy(), "--exportRuntime"])
        .current_dir(&guest_dir)
        .output()
        .map_err(|e| format!("failed to run \"npx asc\": {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "asc failed to compile \"{source_path}\":\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    std::fs::read(&out_path).map_err(|e| format!("cannot read compiled wasm at \"{}\": {e}", out_path.display()))
}

/// Walk upward from `source` looking for a `package.json` that pins
/// `assemblyscript` — the marker of the `guest/` directory `asc` must
/// run from.
fn find_guest_dir(source: &Path) -> Option<std::path::PathBuf> {
    let mut dir = source.parent()?.to_path_buf();
    loop {
        if dir.join("package.json").is_file() && dir.join("node_modules/assemblyscript").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Instantiate `wasm_bytes` under `wasmi`, providing the single
/// `threadbox.ir.v1::emit(ptr, len)` host import, call the guest's
/// exported `main`, and return the bytes the guest emitted as a UTF-8
/// string. See `AGENTS.md` — "The boundary is one row" — this function
/// is the host side of that one row.
fn run_wasm(wasm_bytes: &[u8]) -> Result<String, String> {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, wasm_bytes)
        .map_err(|e| format!("cannot parse wasm module: {e}"))?;

    let mut store = wasmi::Store::new(&engine, Vec::<u8>::new());
    let mut linker = <wasmi::Linker<Vec<u8>>>::new(&engine);

    let emit = wasmi::Func::wrap(
        &mut store,
        |mut caller: wasmi::Caller<'_, Vec<u8>>, ptr: i32, len: i32| {
            let memory = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("guest module has no exported \"memory\"");
            let data = memory.data(&caller);
            let start = ptr as usize;
            let end = start + len as usize;
            let bytes = data[start..end].to_vec();
            caller.data_mut().extend_from_slice(&bytes);
        },
    );
    linker
        .define("threadbox.ir.v1", "emit", emit)
        .map_err(|e| format!("cannot register host import \"threadbox.ir.v1::emit\": {e}"))?;

    // AssemblyScript's `assert()` (used by `emitGraph()`'s single-Publish
    // check) compiles to a call to `env::abort` on failure. The guest
    // never calls it on a well-formed graph, but the import must still
    // be linkable or instantiation fails before `main` ever runs.
    let abort = wasmi::Func::wrap(
        &mut store,
        |_caller: wasmi::Caller<'_, Vec<u8>>, _msg: i32, _file: i32, line: i32, column: i32| -> () {
            panic!("guest module called env::abort at line {line}, column {column} (an internal assertion in ir.ts/emit.ts failed)");
        },
    );
    linker
        .define("env", "abort", abort)
        .map_err(|e| format!("cannot register host import \"env::abort\": {e}"))?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("cannot instantiate wasm module: {e}"))?
        .start(&mut store)
        .map_err(|e| format!("cannot start wasm instance: {e}"))?;

    let main = instance
        .get_typed_func::<(), ()>(&store, "main")
        .map_err(|e| format!("guest module has no exported \"main\": {e}"))?;

    main.call(&mut store, ())
        .map_err(|e| format!("guest \"main\" trapped: {e}"))?;

    String::from_utf8(store.data().clone()).map_err(|e| format!("emitted bytes are not valid UTF-8: {e}"))
}
