// The real composer under the localhost-only native read fixture. This verifies rendering,
// transport defaults and document switching; native batch custody has separate Rust tests.
export const scoreFlowExpression = String.raw`(async () => {
  const wait = async (predicate, label) => {
    const end = performance.now() + 12000;
    while (performance.now() < end) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    throw new Error('timed out: ' + label + ' ' + document.body.innerText.slice(-2000));
  };
  const button = (selector, text) => [...document.querySelectorAll(selector)].find(node => node.textContent.includes(text));
  button('.surface-bar button', 'studio').click();
  await wait(() => button('.studio-obj', 'Moonwalk melody'), 'Score entry');
  button('.studio-obj', 'Moonwalk melody').click();
  await wait(() => document.querySelectorAll('[data-score-cell]').length >= 400, 'real Score grid');
  const cleanGrid = document.querySelectorAll('.sc-lab').length === 25;
  const explicitLoopOff = document.querySelector('[aria-label="Loop playback"]')?.getAttribute('aria-pressed') === 'false';
  button('.studio-obj', 'Recovery sketch').click();
  await wait(() => document.querySelectorAll('[data-score-cell]').length >= 400 &&
    document.querySelector('.studio-obj.active')?.textContent.includes('Recovery sketch') &&
    document.querySelector('[aria-label="Score title"]')?.value === 'Recovery sketch', 'layered recovery Score');
  return { cleanGrid, explicitLoopOff, layeredScoreOpens: true };
})()`;
