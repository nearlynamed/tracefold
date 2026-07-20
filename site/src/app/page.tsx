import Image from "next/image";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { MetricCard } from "@/components/MetricCard";
import { ResultExplorer } from "@/components/ResultExplorer";
import { getPaper, getPublication, getSummary } from "@/lib/results";

const reproduceCommands = `git clone git@github.com:nearlynamed/tracefold.git
cd tracefold

# Fast deterministic correctness + publication path
./scripts/reproduce-smoke.sh

# Pinned ZooKeeper and BGL public evaluation
./scripts/reproduce-public.sh`;

function bytes(value: number): string {
  return value >= 1024 ** 3
    ? `${(value / 1024 ** 3).toFixed(0)} GiB`
    : `${(value / 1024 ** 2).toFixed(0)} MiB`;
}

export default async function HomePage() {
  const [summary, publication, paper] = await Promise.all([
    getSummary(),
    getPublication(),
    getPaper(),
  ]);

  return (
    <>
      <section className="hero shell" id="overview">
        <div className="hero-copy">
          <p className="eyebrow">Executable systems research</p>
          <h1>Keep the answers.<br />Fold the trace.</h1>
          <p className="lede">
            TraceFold tests a deliberately narrow proposition: telemetry archives can retain
            exact declared aggregate answers while discarding old successful payloads.
          </p>
          <div className="hero-actions">
            <a className="button primary" href="#results">Explore results</a>
            <a className="button secondary" href="#paper">Read the report</a>
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

      <section className="claim shell" id="contract">
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

      <div className="shell" id="results">
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

      <section className="paper-section" id="paper" aria-labelledby="paper-title">
        <div className="shell paper-kicker">
          <p className="eyebrow">Full technical report · generated from immutable rows</p>
        </div>
        <article className="paper shell">
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            components={{
              h1: ({ children }) => <h2 className="paper-title" id="paper-title">{children}</h2>,
              h2: ({ children }) => <h3>{children}</h3>,
              h3: ({ children }) => <h4>{children}</h4>,
              img: ({ src, alt }) => {
                if (typeof src !== "string") return null;
                const diagramSource = src.startsWith("./site-data/")
                  ? src.replace("./site-data/", "/generated/")
                  : src;
                return (
                  <span className="paper-diagram">
                    <Image src={diagramSource} alt={alt ?? ""} width={1200} height={650} />
                  </span>
                );
              },
            }}
          >
            {paper}
          </ReactMarkdown>
        </article>
      </section>

      <section className="evidence-section shell" id="evidence" aria-labelledby="evidence-title">
        <div className="section-intro">
          <p className="eyebrow">Machine-readable evidence</p>
          <h2 id="evidence-title">Data and provenance</h2>
          <p className="lede">
            Every table and quantitative sentence above is regenerated from the raw JSONL
            rows below. The site build recomputes each digest before publication.
          </p>
        </div>

        <div className="evidence-grid">
          <div>
            <h3>Publication identity</h3>
            <dl className="metadata-list">
              <div><dt>Benchmark commit</dt><dd><code>{publication.benchmark_commit}</code></dd></div>
              <div><dt>Evidence snapshot</dt><dd><code>{publication.snapshot_id}</code></dd></div>
              <div><dt>Source cap</dt><dd>{summary.max_source_bytes.toLocaleString()} bytes</dd></div>
            </dl>
          </div>

          <div>
            <h3>Raw result files</h3>
            <div className="artifact-list">
              {publication.raw_results.map((artifact) => (
                <article key={artifact.path} className="artifact">
                  <a href={`/generated/${artifact.path}`} download>
                    {artifact.path.split("/").at(-1)}
                  </a>
                  <span>{artifact.bytes.toLocaleString()} bytes</span>
                  <code>sha256:{artifact.sha256}</code>
                </article>
              ))}
            </div>
          </div>
        </div>

        <p className="generated-links">
          Generated: <a href="/generated/summary.json">summary JSON</a>
          {" · "}<a href="/generated/tables/primary.json">primary table</a>
          {" · "}<a href="/generated/methodology.json">methodology</a>
          {" · "}<a href="/generated/CITATION.cff">citation metadata</a>
        </p>
      </section>

      <section className="reproduce-section" id="reproduce" aria-labelledby="reproduce-title">
        <div className="shell reproduce-grid">
          <div>
            <p className="eyebrow">Locked tools · pinned corpora</p>
            <h2 id="reproduce-title">Reproduce the artifact</h2>
            <p className="lede">
              The repository pins Rust, Python, and JavaScript dependencies. Public sources
              are fetched from Loghub, checked against the corpus manifest, and never
              redistributed with the site.
            </p>
            <p>
              The 1 GiB ceiling applies to the downloaded public artifact or generated
              canonical source. Extracted and normalized intermediates may be larger. See the
              {" "}<a href="https://github.com/nearlynamed/tracefold/blob/main/PRD.md">complete protocol</a>
              {" "}for the staged experiment matrix and threats to validity.
            </p>
          </div>
          <pre><code>{reproduceCommands}</code></pre>
        </div>
      </section>
    </>
  );
}
