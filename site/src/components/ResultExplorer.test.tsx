import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ResultExplorer } from "./ResultExplorer";

const rows = [
  {
    dataset: "alpha",
    baseline: "tracefold",
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
  it("switches between published corpora", () => {
    render(<ResultExplorer rows={rows} datasets={["alpha", "beta"]} />);
    expect(screen.getAllByText("tracefold")).toHaveLength(2);
    fireEvent.change(screen.getByLabelText("Corpus"), { target: { value: "beta" } });
    expect(screen.getAllByText("gzip")).toHaveLength(2);
    expect(screen.queryByText("tracefold")).not.toBeInTheDocument();
  });
});
