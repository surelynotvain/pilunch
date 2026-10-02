import { memo, useMemo, useState, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import hljs from "highlight.js/lib/common";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Icon } from "../Icon";
import { parsePathRef, splitMarkdownBlocks } from "../../lib/util";
import { useEditor } from "../../store/editor";
import { useApp } from "../../store/app";

function CodeBlock({ lang, code }: { lang: string; code: string }) {
  const [copied, setCopied] = useState(false);
  const html = useMemo(() => {
    try {
      if (lang && hljs.getLanguage(lang)) return hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
      if (code.length < 20_000) return hljs.highlightAuto(code).value;
    } catch {
      /* fall through to plain text */
    }
    return null;
  }, [lang, code]);
  const copy = () => {
    void navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    });
  };
  return (
    <div className="codeblock">
      <div className="codeblock-head">
        <span>{lang || "text"}</span>
        <span className="spacer" />
        <button className="icon-btn" title="Copy" onClick={copy}>
          <Icon name={copied ? "check" : "copy"} size={14} />
        </button>
      </div>
      <pre>{html != null ? <code dangerouslySetInnerHTML={{ __html: html }} /> : <code>{code}</code>}</pre>
    </div>
  );
}

function InlineCode({ children }: { children?: ReactNode }) {
  const text = String(children ?? "");
  // Select a primitive: a selector returning a fresh object re-renders forever.
  const hasWorkspace = useApp((s) => s.workspace != null);
  const ref = useMemo(() => (hasWorkspace ? parsePathRef(text) : null), [hasWorkspace, text]);
  if (ref) {
    return (
      <code className="path-link" title="Open in editor" onClick={() => void useEditor.getState().openFile(ref.path, ref.line)}>
        {text}
      </code>
    );
  }
  return <code>{text}</code>;
}

const components: Components = {
  // Fenced code is rendered by `pre`; `code` only sees inline code.
  pre({ node }) {
    const codeEl = node?.children?.[0];
    if (codeEl && codeEl.type === "element" && codeEl.tagName === "code") {
      const cls = (codeEl.properties?.className as string[] | undefined)?.find((c) => c.startsWith("language-"));
      const text = codeEl.children.map((c) => (c.type === "text" ? c.value : "")).join("");
      return <CodeBlock lang={cls ? cls.slice(9) : ""} code={text.replace(/\n$/, "")} />;
    }
    return <pre />;
  },
  code({ children }) {
    return <InlineCode>{children}</InlineCode>;
  },
  a({ href, children }) {
    return (
      <a
        href={href}
        onClick={(e) => {
          e.preventDefault();
          if (href && /^https?:\/\//.test(href)) void openUrl(href);
        }}
      >
        {children}
      </a>
    );
  },
};

const remarkPlugins = [remarkGfm];

/** One markdown block; memoized so finished blocks never re-render while text streams. */
const Block = memo(function Block({ text }: { text: string }) {
  return (
    <ReactMarkdown remarkPlugins={remarkPlugins} components={components}>
      {text}
    </ReactMarkdown>
  );
});

export const Markdown = memo(function Markdown({ text }: { text: string }) {
  const blocks = useMemo(() => splitMarkdownBlocks(text), [text]);
  return (
    <div className="md">
      {blocks.map((b, i) => (
        <Block key={i} text={b} />
      ))}
    </div>
  );
});
