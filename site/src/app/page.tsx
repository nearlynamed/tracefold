import Image from "next/image";
import Link from "next/link";
import { MetricCard } from "@/components/MetricCard";
import { ResultExplorer } from "@/components/ResultExplorer";
import { getPublication, getSummary } from "@/lib/results";

function bytes(value: number): string {
  return value >= 1024 ** 3
    ? `${(value / 1024 ** 3).toFixed(0)} GiB`
    : `${(value / 1024 ** 2).toFixed(0)} MiB`;
}

export default async function HomePage() {
  const [summary, publication] = await Promise.all([getSummary(), getPublication()]);

  return (
    <>
      <section className="hero shell">
        <div className="hero-copy">
          <p className="eyebrow">Executable systems research · v1</p>
          <h1>Keep the answers.<br />Fold the trace.</h1>
          <p className="lede">
            TraceFold tests a deliberately narrow proposition: telemetry archives can retain
            exact declared aggregate answers while discarding old successful payloads.
          </p>
          <div className="hero-actions">
            <Link className="button primary" href="#results-heading">Explore results</Link>
            <Link className="button secondary" href="/paper/">Read the report</Link>
          </div>
          <p className="status-line">{publication.status} · by {publication.byline}</p>
        </div>
        <div className="hero-figure">
          <Image
            priority
            src="/opengraph.svg"
            alt="A trace stream folded into compact aggregate blocks"
            width={1200}
            height={630}
          />
        </div>
      </section>

      <section className="metric-grid shell" aria-label="Publication facts">
        <MetricCard label="Benchmark attempts" value={String(summary.attempts)} note={`${summary.successful_attempts} successful`} />
        <MetricCard label="Source cap" value={bytes(summary.max_source_bytes)} note="downloaded or generated source bytes" />
        <MetricCard label="Corpora" value={String(summary.datasets.length)} note={summary.datasets.join(", ")} />
        <MetricCard label="Recorded failures" value={String(summary.failed_attempts)} note="negative results remain published" />
      </section>

      <section className="claim shell">
        <p className="eyebrow">Semantic contract</p>
        <blockquote>
          Recent records and every error remain exact. Older successes survive only as
          deterministic bucketed aggregates for declared query families.
        </blockquote>
        <div className="contract-grid">
          <article><span>01</span><h2>Declare</h2><p>Name dimensions, measures, bucket widths, and retention before encoding.</p></article>
          <article><span>02</span><h2>Fold</h2><p>Validate twice, aggregate deterministically, spill under pressure, and checksum every block.</p></article>
          <article><span>03</span><h2>Prove</h2><p>Compare accepted queries with an exact raw-event oracle and reject undeclared requests.</p></article>
        </div>
      </section>

      <div className="shell">
        <ResultExplorer rows={summary.table} datasets={summary.datasets} />
      </div>

      <section className="limitations shell">
        <div>
          <p className="eyebrow">A constrained claim</p>
          <h2>Compression by giving something up—explicitly.</h2>
        </div>
        <p>
          TraceFold cannot reconstruct old successful payloads or answer arbitrary SQL,
          joins, regex filters, quantiles, or distinct counts. Those are contract boundaries,
          not hidden footnotes. The benchmark is capped at 1 GiB per source artifact and the
          report documents cache, host, normalization, and workload threats to validity.
        </p>
      </section>
    </>
  );
}
