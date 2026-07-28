import { instantiateWasmUrl, requireExports } from "./runtime.mjs";

const REQUIRED_EXPORTS = [
  "normToSentPixel",
  "normToOriginalPixel",
  "planScale",
  "unpackX",
  "unpackY",
];

let runtimePromise;

function loadRuntime() {
  if (!runtimePromise) {
    const url = chrome.runtime.getURL("wasm/coordinates.wasm");
    runtimePromise = instantiateWasmUrl(url).then((instance) =>
      requireExports(instance, REQUIRED_EXPORTS),
    );
  }
  return runtimePromise;
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "threadbox/runtime/ping") {
    sendResponse({ ok: true, runtime: "browser-wasm" });
    return false;
  }

  if (message?.type === "threadbox/runtime/load") {
    loadRuntime().then(
      (instance) => sendResponse({ ok: true, exports: Object.keys(instance.exports) }),
      (error) => sendResponse({ ok: false, error: error.message }),
    );
    return true;
  }

  return false;
});
