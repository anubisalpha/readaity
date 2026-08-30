// Which formats belong to which library (mirrors the Rust `formats` module).

export const COMIC_FORMATS = ["cbz", "cbr"];
export const EBOOK_FORMATS = [
  "epub", "pdf", "mobi", "prc", "azw", "azw3", "txt", "rtf", "lrf",
];

export function isComic(format: string): boolean {
  return COMIC_FORMATS.includes(format);
}

export function isEbook(format: string): boolean {
  return EBOOK_FORMATS.includes(format);
}
