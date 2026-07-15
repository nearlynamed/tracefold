"use client";

import { useMemo, useState } from "react";
import type { ResultRow } from "@/lib/results";

type ResultExplorerProps = {
  rows: ResultRow[];
  datasets: string[];
};

const TRACEFOLD_BASELINE = "tracefold-separate-zstd3";

const number = new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 });

function display(value: number | null, unit = ""): string {
  return value === null ? "—" : `${number.format(value)}${unit}`;
}

export function ResultExplorer({ rows, datasets }: ResultExplorerProps) {
  const [dataset, setDataset] = useState(datasets[0] ?? "");
  const visible = useMemo(
    () => rows.filter((row) => row.dataset === dataset),
    [dataset, rows],
  );
  const maxBytes = Math.max(1, ...visible.map((row) => row.archive_bytes_median ?? 0));

  return (
    <section className="result-explorer" aria-labelledby="results-heading">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Generated evidence</p>
          <h2 id="results-heading">Result explorer</h2>
        </div>
        <label className="dataset-picker">
          Corpus
          <select value={dataset} onChange={(event) => setDataset(event.target.value)}>
            {datasets.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="result-legend" aria-label="Result chart legend">
        <span><i className="legend-swatch tracefold-swatch" aria-hidden="true" />TraceFold <b>ours</b></span>
        <span><i className="legend-swatch baseline-swatch" aria-hidden="true" />Comparison baselines</span>
      </div>

      {visible.length ? (
        <>
          <div className="bars" aria-label={`Median archive size for ${dataset}`}>
            {visible.map((row) => {
              const isTraceFold = row.baseline === TRACEFOLD_BASELINE;
              return (
              <div
                className={`bar-row${isTraceFold ? " is-tracefold" : ""}`}
                key={`${row.dataset}-${row.baseline}`}
              >
                <span className="bar-label">
                  {isTraceFold ? <>TraceFold <span className="ours-badge">ours</span></> : row.baseline}
                </span>
                <div className="bar-track">
                  <div
                    className="bar-fill"
                    style={{ width: `${Math.max(2, ((row.archive_bytes_median ?? 0) / maxBytes) * 100)}%` }}
                  />
                </div>
                <strong className="bar-value">{display(row.archive_bytes_median, " B")}</strong>
              </div>
              );
            })}
          </div>

          <div className="table-scroll">
            <table>
              <caption>Median measurements for {dataset}</caption>
              <thead>
                <tr>
                  <th scope="col">Archive</th>
                  <th scope="col">Attempts</th>
                  <th scope="col">Compression</th>
                  <th scope="col">Encode</th>
                  <th scope="col">Query batch</th>
                </tr>
              </thead>
              <tbody>
                {visible.map((row) => {
                  const isTraceFold = row.baseline === TRACEFOLD_BASELINE;
                  return (
                  <tr
                    className={isTraceFold ? "is-tracefold" : undefined}
                    key={`${row.dataset}-${row.baseline}`}
                  >
                    <th scope="row">
                      {isTraceFold ? <>TraceFold <span className="ours-badge">ours</span></> : row.baseline}
                    </th>
                    <td>{row.attempts}</td>
                    <td>{display(row.compression_ratio_median, "×")}</td>
                    <td>{display(row.encode_wall_ns_median === null ? null : row.encode_wall_ns_median / 1e6, " ms")}</td>
                    <td>{display(row.query_wall_ns_median === null ? null : row.query_wall_ns_median / 1e6, " ms")}</td>
                  </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </>
      ) : (
        <p className="empty-state">No successful benchmark rows were published for this corpus.</p>
      )}
    </section>
  );
}
