/// Pure coordinate algebra compiled to WebAssembly. No I/O lives here:
/// no `fetch`, Canvas, or DOM. A browser tool may own screenshot capture
/// and scaling; this module owns the deterministic inverse mapping from
/// normalized `[0,1000]` coordinates into the original pixel frame.
///
/// The exported API takes only plain integers (no classes cross the
/// WASM boundary — AssemblyScript classes are not exported as
/// constructors under `--exportRuntime`). Results that are pairs are
/// packed into one i64: high 32 bits = x, low 32 bits = y.

/// Convert a normalized `[0,1000]` coordinate into a pixel
/// coordinate on the *sent* (padded, scaled) canvas. Pixel coordinates
/// are indices, so the inclusive normalized endpoint 1000 maps to the
/// final index `dimension - 1`. Returns -1 for an invalid normalized
/// coordinate or a non-positive dimension.
export function normToSentPixel(norm: i32, dimension: i32): i32 {
  if (norm < 0 || norm > 1000 || dimension <= 0) return -1;
  return (norm * (dimension - 1) + 500) / 1000;
}

/// The full inverse map: take normalized `{x,y}` plus the
/// description of how the host scaled+padded the original capture
/// before sending, and return the corresponding pixel in the original
/// page capture's coordinate frame. Returns -1 (as an i64) if the
/// normalized coordinate lands in the pad.
///
/// Parameters:
///   normX, normY       — normalized [0,1000] coordinates
///   canvasWidth        — full sent canvas width, including pad
///   canvasHeight       — full sent canvas height, including pad
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
  if (
    canvasWidth <= 0 ||
    canvasHeight <= 0 ||
    contentWidth <= 0 ||
    contentHeight <= 0 ||
    origWidth <= 0 ||
    origHeight <= 0
  ) {
    return -1;
  }
  const sentX = normToSentPixel(normX, canvasWidth);
  const sentY = normToSentPixel(normY, canvasHeight);
  if (sentX < 0 || sentY < 0) return -1;

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
  if (origWidth <= 0 || origHeight <= 0 || targetWidth <= 0 || targetHeight <= 0) {
    return -1;
  }
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
