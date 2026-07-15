import type { Metadata } from "next";
import { getPublication, getSummary } from "@/lib/results";

export const metadata: Metadata = { title: "Data" };

export default async function DataPage() {
  const [summary, publication] = await Promise.all([getSummary(), getPublication()]);

  return (
    <article className="prose-page shell">
      <p className="eyebrow">Machine-readable evidence</p>
      <h1>Data and provenance</h1>
      <p className="lede">
        Every chart and table on this site is generated from the raw JSONL benchmark rows
        listed below. Downloads include hashes so the publication can be audited independently.
      </p>

      <section>
        <h2>Publication identity</h2>
        <dl className="metadata-list">
          <div><dt>Benchmark commit</dt><dd><code>{publication.benchmark_commit}</code></dd></div>
          <div><dt>Publication commit</dt><dd><code>{publication.publication_commit}</code></dd></div>
          <div><dt>Source cap</dt><dd>{summary.max_source_bytes.toLocaleString()} bytes</dd></div>
        </dl>
      </section>

      <section>
        <h2>Raw result files</h2>
        <div className="artifact-list">
          {publication.raw_results.map((artifact) => (
            <article key={artifact.path} className="artifact">
              <a href={`/generated/${artifact.path}`} download>{artifact.path.split("/").at(-1)}</a>
              <span>{artifact.bytes.toLocaleString()} bytes</span>
              <code>sha256:{artifact.sha256}</code>
            </article>
          ))}
        </div>
      </section>

      <section>
        <h2>Generated tables</h2>
        <p><a href="/generated/summary.json">Summary JSON</a> · <a href="/generated/tables/primary.json">Primary table JSON</a> · <a href="/generated/methodology.json">Methodology JSON</a></p>
      </section>
    </article>
  );
}
