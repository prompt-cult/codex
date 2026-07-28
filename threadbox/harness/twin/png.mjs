/// A minimal, dependency-free PNG encoder.
///
/// The twin renders its own screenshots so that a capture has real image bytes
/// with a stable content hash. That matters for two reasons: the fixture model
/// corpus is keyed by the artifact hash, and a run must be replayable, so the
/// same page state has to produce the same bytes every time.
import { deflateSync } from "node:zlib";

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([length, body, crc]);
}

/// Encode an RGB pixel buffer (`width * height * 3` bytes) as a PNG.
export function encodePng(width, height, rgb) {
  const stride = width * 3;
  // Each scanline is prefixed with filter type 0 (None) — no filtering keeps
  // the output byte-for-byte reproducible without depending on a heuristic.
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    rgb.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // colour type: truecolour
  ihdr[10] = 0; // deflate
  ihdr[11] = 0; // adaptive filtering
  ihdr[12] = 0; // no interlace

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/// A tiny drawing surface: enough to render recognisable form furniture.
export function surface(width, height, background = [255, 255, 255]) {
  const rgb = Buffer.alloc(width * height * 3);
  for (let i = 0; i < width * height; i++) {
    rgb[i * 3] = background[0];
    rgb[i * 3 + 1] = background[1];
    rgb[i * 3 + 2] = background[2];
  }
  return {
    width,
    height,
    rgb,
    rect(x, y, w, h, [r, g, b]) {
      const x0 = Math.max(0, Math.round(x));
      const y0 = Math.max(0, Math.round(y));
      const x1 = Math.min(width, Math.round(x + w));
      const y1 = Math.min(height, Math.round(y + h));
      for (let py = y0; py < y1; py++) {
        for (let px = x0; px < x1; px++) {
          const i = (py * width + px) * 3;
          rgb[i] = r;
          rgb[i + 1] = g;
          rgb[i + 2] = b;
        }
      }
    },
    toPng() {
      return encodePng(width, height, rgb);
    },
  };
}
