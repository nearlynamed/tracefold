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

if (publication.schema_version !== 1 || publication.byline !== "nearlynamed") {
  throw new Error("results/site-data/publication.json has an unsupported schema or byline");
}

for (const artifact of publication.raw_results ?? []) {
  if (!artifact.path || !artifact.sha256) {
    throw new Error("every published raw artifact needs a path and SHA-256 digest");
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
