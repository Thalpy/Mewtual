<script lang="ts">
  import { dismissOnBackdrop } from "./overlay-dismiss";

  let { onclose } = $props<{ onclose: () => void }>();
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="overlay" role="presentation" use:dismissOnBackdrop={onclose}>
  <div class="overlay-card">
    <header class="overlay-head">
      <h2>Wiki formatting</h2>
      <button class="ghost" onclick={onclose}>✕</button>
    </header>
    <div class="overlay-body wiki-help">
      <p>Each page is written in <strong>Markdown</strong> or <strong>Wikitext</strong>: pick per page with the
        <code>md / wiki</code> switch in Edit mode. The choice is a page property shared with every member.
        Pages with 3+ headings get an automatic <strong>Contents</strong> box.</p>
      <h3>Link to another page (both formats)</h3>
      <p><code>[[Getting Started]]</code>, or with display text: <code>[[Getting Started|the guide]]</code>.
        Click a link to open it; a <span class="wikilink missing">red link</span> means the page doesn't exist
        yet: click it to create it.</p>
      <h3>Embed an image / video / audio (both formats)</h3>
      <p>In Edit mode, <strong>drag a file onto the editor</strong> or use the 📎 button. It's stored in the
        fileshare under <code>wiki/&lt;page&gt;/</code> and shown inline.</p>
      <h3>Infobox (both formats)</h3>
      <p>The summary card that floats at the top right of a page. Write one block, anywhere on the
        page, with the <code>▤</code> toolbar button; <code>title</code>, <code>image</code> and
        <code>caption</code> are the card's own chrome, every other line is a row, and a line with an
        empty value becomes a section band. One infobox per page.</p>
      <pre class="wiki-help-block">{`{{Infobox
| title   = Whiskers
| image   = (use 📎 or + insert to place a file here)
| caption = At the cafe
| Species = Cat
| Owner   = [[Alice]]
| Details =
| Age     = 4
}}`}</pre>
      <h3>Markdown pages</h3>
      <ul>
        <li><code>**bold**</code>, <code>*italic*</code>, <code>`code`</code></li>
        <li><code># Heading</code>, <code>## Subheading</code></li>
        <li><code>- bullet</code> lists, <code>1. numbered</code> lists</li>
        <li><code>&gt; quote</code>, <code>---</code> divider, <code>[text](https://link)</code></li>
      </ul>
      <h3>Wikitext pages</h3>
      <ul>
        <li><code>'''bold'''</code>, <code>''italic''</code>, <code>'''''both'''''</code></li>
        <li><code>== Heading ==</code>, <code>=== Subheading ===</code></li>
        <li><code>* bullet</code> / <code># numbered</code> lists; nest by repeating (<code>**</code>, <code>##</code>)</li>
        <li><code>; term : definition</code>, <code>:</code> indent, <code>----</code> divider</li>
        <li><code>[https://link label]</code> external link</li>
        <li><code>{"{|"}</code> … <code>{"|}"}</code> table, with <code>|-</code> rows, <code>!</code> header cells, <code>|+</code> caption</li>
        <li><code>&lt;nowiki&gt;…&lt;/nowiki&gt;</code> shows markup literally</li>
      </ul>
      <h3>Page tools</h3>
      <ul>
        <li><strong>Contents box</strong>: automatic at 3+ headings; force with <code>__TOC__</code>, suppress with <code>__NOTOC__</code>.</li>
        <li><strong>Sections</strong>: hover a heading in Read mode for a per-section <em>edit</em> jump.</li>
        <li><strong>Redirects</strong>: a page whose first line is <code>#REDIRECT [[Target]]</code> forwards readers there.</li>
        <li><strong>Rename / delete</strong>: in the page header (rename doesn't rewrite links: old links go red).</li>
        <li><strong>What links here</strong>: pages linking to the open page, listed under it.</li>
      </ul>
    </div>
  </div>
</div>

<style>
  .wiki-help h3 {
    margin: 0.8rem 0 0.3rem;
    font-size: 0.9rem;
  }
  .wiki-help code {
    background: var(--bg-0);
    border: 1px solid var(--border-soft);
    border-radius: var(--r);
    padding: 0 0.3em;
  }
  .wiki-help-block {
    margin: 0.4rem 0 0.8rem;
    padding: 0.55rem 0.7rem;
    background: var(--bg-0);
    border: 1px solid var(--bg-elev);
    border-radius: 4px;
    font-family: var(--mono);
    font-size: 0.75rem;
    line-height: 1.5;
    white-space: pre;
    overflow-x: auto;
    color: var(--text-2);
  }
</style>
