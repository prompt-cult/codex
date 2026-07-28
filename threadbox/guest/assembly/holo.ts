/// Pure coordinate algebra for the Holo vision round-trip, compiled to
/// WebAssembly. No I/O lives here — no `fetch`, no Canvas, no DOM. The
/// browser host (`harness/dummy-form/holo.js`) owns capture, scaling,
/// and the network call; this module owns the deterministic math that
/// turns Holo's normalized `[0,1000]` coordinates back into the page's
/// original pixel coordinates, inverting whatever scale-and-pad the
/// host applied before sending the image.
///
/// The key property, established by spike07/09 of the stenography repo
/// and re-proven live against `holo3-1-35b-a3b`: Holo returns
/// coordinates normalized to the image it was *sent*, not to any
/// canonical resolution. So scaling the image before sending is safe
/// as long as the inverse map undoes exactly the scale and the pad.
/// See `harness/dummy-form/README.md` for the full proof transcript.
///
/// The exported API takes only plain integers (no classes cross the
/// WASM boundary — AssemblyScript classes are not exported as
/// constructors under `--exportRuntime`). Results that are pairs are
/// packed into one i64: high 32 bits = x, low 32 bits = y.

/// Convert Holo's normalized `[0,1000]` coordinate into a pixel
/// coordinate on the *sent* (padded, scaled) canvas. This is the first
/// half of the round-trip and the only step spike07/09 needed, because
/// they sent the raw image with no pad. Round-half-up so 0 maps to 0
/// and 1000 maps to exactly `dimension`.
export function normToSentPixel(norm: i32, dimension: i32): i32 {
  return (norm * dimension + 500) / 1000;
}

/// The full inverse map: take Holo's normalized `{x,y}` plus the
/// description of how the host scaled+padded the original capture
/// before sending, and return the corresponding pixel in the original
/// page capture's coordinate frame. Returns -1 (as an i64) if the
/// normalized coordinate lands in the pad.
///
/// Parameters:
///   normX, normY       — Holo's [0,1000] coordinates
///   canvasWidth        — full canvas width sent to Holo, including pad
///   canvasHeight       — full canvas height sent to Holo, including pad
///   contentWidth       — scaled screenshot width inside the canvas
///   contentHeight      — scaled screenshot height inside the canvas
///   offsetX, offsetY   — content offset inside the canvas (the pad)
///   origWidth, origHeight — the original page capture dimensions
export function normToOriginalPixel(
  normX: i32,
  normY: i32,
  canvasWidth: i32,
  canvasHeight: i32,
  contentWidth: i32,
  contentHeight: i32,
  offsetX: i32,
  offsetY: i32,
  origWidth: i32,
  origHeight: i32,
): i64 {
  const sentX = normToSentPixel(normX, canvasWidth);
  const sentY = normToSentPixel(normY, canvasHeight);

  const inContentX = sentX - offsetX;
  const inContentY = sentY - offsetY;

  if (inContentX < 0 || inContentY < 0 || inContentX >= contentWidth || inContentY >= contentHeight) {
    return -1;
  }

  // Un-scale: content was scaled from origWidth -> contentWidth.
  const origX = (inContentX * origWidth + contentWidth / 2) / contentWidth;
  const origY = (inContentY * origHeight + contentHeight / 2) / contentHeight;

  return ((origX as i64) << 32) | (origY as i64);
}

/// Compute the scaled content dimensions for a top-left placement of
/// an `origWidth x origHeight` capture inside a `targetWidth x
/// targetHeight` canvas, preserving aspect ratio. Returns one i64:
/// high32 = contentWidth, low32 = contentHeight.
export function planScale(origWidth: i32, origHeight: i32, targetWidth: i32, targetHeight: i32): i64 {
  let contentWidth = targetWidth;
  let contentHeight = (origHeight * targetWidth + origWidth / 2) / origWidth;
  if (contentHeight > targetHeight) {
    contentHeight = targetHeight;
    contentWidth = (origWidth * targetHeight + origHeight / 2) / origHeight;
  }
  return ((contentWidth as i64) << 32) | (contentHeight as i64);
}

/// Unpack the x component of a packed `(x,y)` i64.
export function unpackX(packed: i64): i32 {
  return (packed >>> 32) as i32;
}

/// Unpack the y component of a packed `(x,y)` i64.
export function unpackY(packed: i64): i32 {
  return (packed & 0xFFFFFFFF) as i32;
}
