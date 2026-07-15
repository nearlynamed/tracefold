import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ResultExplorer } from "./ResultExplorer";

const rows = [
  {
    dataset: "alpha",
    baseline: "tracefold-separate-zstd3",
    attempts: 3,
    archive_bytes_median: 100,
    compression_ratio_median: 5,
    encode_wall_ns_median: 2_000_000,
    query_wall_ns_median: 1_000_000,
    encode_wall_ns_p95: 3_000_000,
  },
  {
    dataset: "beta",
    baseline: "gzip",
    attempts: 2,
    archive_bytes_median: 200,
    compression_ratio_median: 2,
    encode_wall_ns_median: 4_000_000,
    query_wall_ns_median: 3_000_000,
    encode_wall_ns_p95: 5_000_000,
  },
];

describe("ResultExplorer", () => {
  it("switches corpora and makes TraceFold visibly distinct", () => {
    const { container } = render(<ResultExplorer rows={rows} datasets={["alpha", "beta"]} />);
    expect(screen.getByLabelText("Result chart legend")).toHaveTextContent("TraceFold ours");
    expect(container.querySelectorAll(".is-tracefold")).toHaveLength(2);
    expect(screen.getAllByText("ours")).toHaveLength(3);
    fireEvent.change(screen.getByLabelText("Corpus"), { target: { value: "beta" } });
    expect(screen.getAllByText("gzip")).toHaveLength(2);
    expect(container.querySelectorAll(".is-tracefold")).toHaveLength(0);
  });
});
