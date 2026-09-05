/**
 * The search scan, off the UI thread.
 *
 * It holds the corpus so a query does not re-send it, and answers with hit references rather than
 * messages so the reply stays small whatever the result size. Ordering and naming stay on the
 * main thread, which owns profiles.
 *
 * Same origin, same process sandbox: this moves plaintext no further than the tab it is already
 * in. It is cleared with the corpus, and the whole worker goes away when search closes.
 */

import { findMatches, type SearchCorpusChannel, type SearchRequest } from "./search-index.ts";

let corpus: SearchCorpusChannel[] = [];

self.onmessage = (event: MessageEvent<SearchRequest>) => {
  const request = event.data;
  if (request.type === "corpus") {
    corpus = request.corpus;
    return;
  }
  if (request.type === "query") {
    self.postMessage({ type: "result", id: request.id, hits: findMatches(corpus, request.spec) });
  }
};
