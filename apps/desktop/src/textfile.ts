// Which shared files the Files tab's Properties pane can show as readable text.
//
// The pane already previews image/video/audio inline (`safeMime` in App.svelte). Documents and
// source files are the other half of "can I look at this without saving it first": a shared
// README, a config, a log, a patch. Markdown additionally gets a rendered view, so a `.md`
// dropped into a folder reads like a page instead of like its own source.
//
// Everything here is PURE (no DOM, no `invoke`), so it unit-tests under plain Node like
// `wikitext.ts`. The MIME an uploader stamps on a listing is theirs to choose, so the extension
// is trusted first and the MIME is only a fallback; neither decides anything more dangerous
// than which viewer the pane opens.

/** "" = not a text file: the pane shows no reader for it. */
export type TextFileKind = "markdown" | "text" | "";

// Inline reading is a convenience, not a substitute for downloading: a listing can declare up to
// `MAX_FILE_BYTES` (256 MiB), and `download_file` hands the whole blob back as one base64 string.
// Anything past this is fetched only when the reader explicitly asks.
export const TEXT_PREVIEW_MAX_BYTES = 2 * 1024 * 1024;

const MARKDOWN_EXT = new Set(["md", "markdown", "mdown", "mkd", "mkdn", "mdwn", "mdtext"]);

// Plain-text extensions: documents, data/config formats, and the source languages someone is
// plausibly sharing. Rendered as source only; nothing here is ever interpreted.
const TEXT_EXT = new Set([
  // documents and logs
  "txt", "text", "log", "nfo", "me", "rst", "adoc", "asciidoc", "org", "tex", "srt", "vtt",
  // data and config
  "csv", "tsv", "json", "jsonc", "json5", "ndjson", "yaml", "yml", "toml", "ini", "cfg", "conf",
  "properties", "env", "xml", "plist", "csproj", "gradle", "lock", "gitignore", "gitattributes",
  "editorconfig", "dockerignore", "npmrc", "nvmrc", "prettierrc", "eslintrc", "babelrc",
  // markup and styling (source view: never rendered)
  "html", "htm", "xhtml", "css", "scss", "sass", "less", "styl", "svelte", "vue",
  // source
  "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "rs", "py", "pyi", "rb", "go", "java",
  "kt", "kts", "scala", "swift", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "cs", "m", "mm",
  "php", "pl", "pm", "lua", "r", "jl", "dart", "ex", "exs", "erl", "hrl", "hs", "elm", "clj",
  "cljs", "fs", "fsx", "vb", "asm", "s", "zig", "nim", "v", "sql", "graphql", "gql", "proto",
  // shells and build files
  "sh", "bash", "zsh", "fish", "ps1", "psm1", "psd1", "bat", "cmd", "mk", "cmake", "nix",
  // patches
  "diff", "patch",
]);

// Extension-less files that are text by convention. Matched on the whole (lowercased) name with
// any leading dot stripped, so `.gitignore` and `LICENSE` both land here.
const TEXT_BASENAMES = new Set([
  "readme", "license", "licence", "copying", "notice", "changelog", "changes", "authors",
  "contributors", "install", "todo", "makefile", "dockerfile", "gitignore", "gitattributes",
  "gitmodules", "editorconfig", "dockerignore", "npmrc", "nvmrc", "procfile", "codeowners",
  "gemfile", "rakefile", "cargo", "vagrantfile", "jenkinsfile",
]);

// MIME types that mean text but do not start with `text/`.
const TEXT_MIME = new Set([
  "application/json", "application/ld+json", "application/xml", "application/xhtml+xml",
  "application/javascript", "application/x-javascript", "application/ecmascript",
  "application/typescript", "application/x-typescript", "application/yaml",
  "application/x-yaml", "application/toml", "application/x-toml", "application/sql",
  "application/x-sh", "application/x-shellscript", "application/x-perl", "application/x-python",
  "application/x-ruby", "application/x-httpd-php", "application/x-tex", "application/graphql",
]);

/** The lowercased extension after the final dot, or "" for a dotfile / an extension-less name. */
function extensionOf(name: string): string {
  const base = (name || "").split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  if (dot <= 0 || dot === base.length - 1) return "";
  return base.slice(dot + 1).toLowerCase();
}

/**
 * Which text viewer (if any) the Properties pane should offer for a listing.
 *
 * The extension decides when it is one we know, because it is what the author actually named the
 * file; the declared MIME only fills the gap. Media (image/video/audio) never reaches here: the
 * caller already has an inline player for those, and two previews of one file is just noise.
 */
export function textFileKind(name: string, mime: string): TextFileKind {
  const ext = extensionOf(name);
  if (MARKDOWN_EXT.has(ext)) return "markdown";
  if (TEXT_EXT.has(ext)) return "text";

  const base = ((name || "").split(/[\\/]/).pop() ?? "").toLowerCase().replace(/^\.+/, "");
  if (!ext && TEXT_BASENAMES.has(base)) return "text";

  // `text/markdown; charset=utf-8` and friends: the parameters are not part of the decision.
  const type = (mime || "").split(";")[0].trim().toLowerCase();
  if (!type) return "";
  if (type === "text/markdown" || type === "text/x-markdown") return "markdown";
  if (type.startsWith("image/") || type.startsWith("video/") || type.startsWith("audio/")) return "";
  if (type.startsWith("text/")) return "text";
  if (TEXT_MIME.has(type)) return "text";
  // Structured suffixes (`application/vnd.foo+json`, `…+xml`) are text by definition.
  if (/\+(json|xml|yaml)$/.test(type)) return "text";
  return "";
}

export type TextDecode =
  | { ok: true; text: string; lines: number }
  | { ok: false; reason: "binary" };

/**
 * Decode a shared blob for reading. Honours a UTF-8/UTF-16 byte-order mark, otherwise insists on
 * valid UTF-8; anything else (or any embedded NUL, which valid UTF-8 permits but no text file
 * carries) is reported as binary rather than shown as replacement-character soup.
 *
 * Line endings are normalised to `\n` so a CRLF file does not render with stray carriage returns
 * inside the `<pre>`.
 */
export function decodeTextFile(bytes: Uint8Array): TextDecode {
  let body = bytes;
  let label = "utf-8";
  if (bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
    body = bytes.subarray(3);
  } else if (bytes.length >= 2 && bytes[0] === 0xff && bytes[1] === 0xfe) {
    body = bytes.subarray(2);
    label = "utf-16le";
  } else if (bytes.length >= 2 && bytes[0] === 0xfe && bytes[1] === 0xff) {
    body = bytes.subarray(2);
    label = "utf-16be";
  }
  let text: string;
  try {
    text = new TextDecoder(label, { fatal: true }).decode(body);
  } catch {
    return { ok: false, reason: "binary" };
  }
  if (text.includes("\0")) return { ok: false, reason: "binary" };
  text = text.replace(/\r\n?/g, "\n");
  return { ok: true, text, lines: text ? text.split("\n").length : 0 };
}

/** "1,204 lines" / "1 line", for the reader's toolbar. */
export function lineCountLabel(lines: number): string {
  return `${lines.toLocaleString()} line${lines === 1 ? "" : "s"}`;
}
