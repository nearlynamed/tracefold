import type { Metadata } from "next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { getPaper } from "@/lib/results";

export const metadata: Metadata = { title: "Paper" };

export default async function PaperPage() {
  const paper = await getPaper();
  return (
    <article className="paper shell">
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{paper}</ReactMarkdown>
    </article>
  );
}
