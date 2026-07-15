import type { Metadata } from "next";

export const metadata: Metadata = { title: "Reproduce" };

const commands = `git clone git@github.com:nearlynamed/tracefold.git
cd tracefold
cargo test --workspace
cargo run --release -p tracefold-cli -- bench fetch --manifest benches/corpora.toml
cargo run --release -p tracefold-cli -- bench smoke --output results/raw/smoke.jsonl
uv run --project scripts/report tracefold-report build results/raw --output results
pnpm install --frozen-lockfile
pnpm --dir site build`;

export default function ReproducePage() {
  return (
    <article className="prose-page shell">
      <p className="eyebrow">Locked tools · immutable rows</p>
      <h1>Reproduce the artifact</h1>
      <p className="lede">
        The repository pins Rust, Python, and JavaScript dependencies. Benchmark data is
        fetched from its original public source and never redistributed here.
      </p>
      <section>
        <h2>Fast verification path</h2>
        <pre><code>{commands}</code></pre>
      </section>
      <section>
        <h2>Interpret the cap correctly</h2>
        <p>
          The 1 GiB ceiling applies to the downloaded public artifact or generated canonical
          JSONL source. Extracted and normalized intermediates may be larger. A successful row
          whose source exceeds the declared limit is rejected by the report generator.
        </p>
      </section>
      <section>
        <h2>Full protocol</h2>
        <p>
          See the repository README and <a href="https://github.com/nearlynamed/tracefold/blob/main/PRD.md">PRD</a>
          {" "}for corpus hashes, trial counts, workload construction, semantic checks, and
          known threats to validity.
        </p>
      </section>
    </article>
  );
}
