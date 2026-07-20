import { cp, mkdir, readFile, rm } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const source = path.join(root, "results/site-data");
const destination = path.join(root, "site/public/generated");

const publication = JSON.parse(
  await readFile(path.join(source, "publication.json"), "utf8"),
);
const summary = JSON.parse(await readFile(path.join(source, "summary.json"), "utf8"));
const methodology = JSON.parse(
  await readFile(path.join(source, "methodology.json"), "utf8"),
);
const paper = await readFile(path.join(root, "results/paper.md"), "utf8");

if (publication.schema_version !== 1 || publication.byline !== "nearlynamed") {
  throw new Error("results/site-data/publication.json has an unsupported schema or byline");
}
if (
  !publication.snapshot_id ||
  publication.snapshot_id !== summary.snapshot_id ||
  publication.snapshot_id !== methodology.snapshot_id ||
  !paper.includes(publication.snapshot_id)
) {
  throw new Error("generated findings do not share one evidence snapshot");
}

const declared = new Set(publication.raw_results.map((artifact) => artifact.path));
const expectedEvidence = new Set(
  summary.table.map((row, index) => `table-primary-${index}:${row.dataset}:${row.baseline}`),
);
const observedEvidence = new Set(
  publication.evidence.map((row) => `${row.id}:${row.dataset}:${row.baseline}`),
);
if (
  expectedEvidence.size !== observedEvidence.size ||
  [...expectedEvidence].some((entry) => !observedEvidence.has(entry))
) {
  throw new Error("publication evidence does not match the primary findings table");
}

for (const artifact of publication.raw_results ?? []) {
  if (!artifact.path || !artifact.sha256) {
    throw new Error("every published raw artifact needs a path and SHA-256 digest");
  }
  if (!artifact.path.startsWith("raw/") || declared.size !== publication.raw_results.length) {
    throw new Error("published raw artifact paths must be unique and remain under raw/");
  }

  const contents = await readFile(path.join(source, artifact.path));
  const digest = createHash("sha256").update(contents).digest("hex");
  if (digest !== artifact.sha256) {
    throw new Error(`SHA-256 mismatch for ${artifact.path}`);
  }
}

await rm(destination, { recursive: true, force: true });
await mkdir(destination, { recursive: true });
await cp(source, destination, { recursive: true });
await cp(path.join(root, "results/paper.md"), path.join(destination, "paper.md"));
await cp(path.join(root, "results/summary.md"), path.join(destination, "summary.md"));
await cp(path.join(root, "paper/CITATION.cff"), path.join(destination, "CITATION.cff"));
