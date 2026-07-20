import { readFile } from "node:fs/promises";
import path from "node:path";

export type ResultRow = {
  dataset: string;
  baseline: string;
  attempts: number;
  archive_bytes_median: number | null;
  compression_ratio_median: number | null;
  encode_wall_ns_median: number | null;
  query_wall_ns_median: number | null;
  encode_wall_ns_p95: number | null;
};

export type Summary = {
  schema_version: 1;
  snapshot_id: string;
  attempts: number;
  successful_attempts: number;
  failed_attempts: number;
  datasets: string[];
  baselines: string[];
  max_source_bytes: number;
  table: ResultRow[];
  failures: Array<{
    dataset: string;
    baseline: string;
    kind: string | null;
    error: string | null;
  }>;
  charts: string[];
};

export type Publication = {
  schema_version: 1;
  title: string;
  byline: string;
  status: string;
  benchmark_commit: string;
  snapshot_id: string;
  raw_results: Array<{ path: string; bytes: number; sha256: string }>;
};

async function generatedText(name: string): Promise<string> {
  return readFile(path.join(process.cwd(), "public", "generated", name), "utf8");
}

export async function getSummary(): Promise<Summary> {
  return JSON.parse(await generatedText("summary.json")) as Summary;
}

export async function getPublication(): Promise<Publication> {
  return JSON.parse(await generatedText("publication.json")) as Publication;
}

export async function getPaper(): Promise<string> {
  return generatedText("paper.md");
}
