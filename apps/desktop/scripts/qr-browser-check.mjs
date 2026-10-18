// Dev-server interop regression: production Rollup and Node's package loader cannot verify
// Vite's browser prebundles. Exercise both lazy CommonJS dependencies without camera access.
import QRCode from "qrcode";
import jsQR from "jsqr";

export async function verifyQrInterop() {
  const text = "Mewtual";
  const qr = QRCode.create(text, { version: 1, errorCorrectionLevel: "L" });
  const quiet = 4, scale = 4, size = (qr.modules.size + quiet * 2) * scale;
  const pixels = new Uint8ClampedArray(size * size * 4).fill(255);
  for (let y = 0; y < qr.modules.size; y++) {
    for (let x = 0; x < qr.modules.size; x++) {
      if (!qr.modules.data[y * qr.modules.size + x]) continue;
      for (let dy = 0; dy < scale; dy++) {
        for (let dx = 0; dx < scale; dx++) {
          const offset = (((y + quiet) * scale + dy) * size + (x + quiet) * scale + dx) * 4;
          pixels[offset] = pixels[offset + 1] = pixels[offset + 2] = 0;
        }
      }
    }
  }
  const decoded = jsQR(pixels, size, size, { inversionAttempts: "dontInvert" });
  const png = await QRCode.toDataURL(text, { version: 1, errorCorrectionLevel: "L" });
  return { png: png.startsWith("data:image/png;base64,"), decoded: decoded?.data ?? null };
}
