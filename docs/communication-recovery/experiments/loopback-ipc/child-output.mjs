/** `exit` does not mean captured stdout/stderr reached EOF. Evidence in those pipes belongs
 * to this later close event; the caller supplies its original bounded wait and child cleanup. */
export function childOutputClosed(child) {
  return new Promise(resolve => child.once("close", (code, signal) => resolve({ code, signal })));
}
