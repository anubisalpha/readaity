// Render a PDF's first page to a JPEG cover and cache it in the DB, so the
// shelf shows PDF covers without opening each file (parity with comics).

import * as pdfjsLib from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { readBookBytes, setCover } from "./api";

pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl;

/** Returns a data URL for the generated cover, or null on failure. */
export async function generatePdfCover(path: string): Promise<string | null> {
  try {
    const bin = atob(await readBookBytes(path));
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);

    const doc = await pdfjsLib.getDocument({ data: bytes }).promise;
    const pg = await doc.getPage(1);
    const base = pg.getViewport({ scale: 1 });
    const scale = Math.min(360 / base.width, 540 / base.height);
    const vp = pg.getViewport({ scale });

    const canvas = document.createElement("canvas");
    canvas.width = vp.width;
    canvas.height = vp.height;
    await pg.render({
      canvas,
      canvasContext: canvas.getContext("2d")!,
      viewport: vp,
    }).promise;

    const dataUrl = canvas.toDataURL("image/jpeg", 0.8);
    await setCover(path, dataUrl.split(",")[1], vp.width, vp.height);
    return dataUrl;
  } catch {
    return null;
  }
}
