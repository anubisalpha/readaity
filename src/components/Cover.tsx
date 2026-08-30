import { useEffect, useRef, useState } from "react";
import { getCoverUrl } from "../lib/api";
import { generatePdfCover } from "../lib/pdfCover";

interface Props {
  path: string;
  title: string;
  format: string;
}

/**
 * A shelf cover: lazily fetches the cached thumbnail (built during the sweep and
 * stored in the DB) once scrolled near the viewport, so a big library doesn't
 * decode every cover up front.
 */
export function Cover({ path, title, format }: Props) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let cancelled = false;

    const load = async () => {
      try {
        let u = await getCoverUrl(path);
        // No cached cover + it's a PDF → render one via pdf.js and cache it.
        if (!u && format === "pdf") u = await generatePdfCover(path);
        if (!cancelled) {
          if (u) setUrl(u);
          else setFailed(true);
        }
      } catch {
        if (!cancelled) setFailed(true);
      }
    };

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer.disconnect();
          load();
        }
      },
      { rootMargin: "200px" },
    );
    observer.observe(el);

    return () => {
      cancelled = true;
      observer.disconnect();
    };
  }, [path]);

  return (
    <div className="cover" ref={ref}>
      {url ? (
        <img src={url} alt={title} loading="lazy" />
      ) : (
        <div className={`cover-placeholder${failed ? " failed" : ""}`}>
          <span>{failed ? "⚠" : title}</span>
        </div>
      )}
    </div>
  );
}
