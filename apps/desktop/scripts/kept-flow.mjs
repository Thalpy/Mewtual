// Rendered regression for the real App's retained-copy controls. IPC is mocked at the fixture
// boundary; this proves UI wiring/ownership, not native persistence, network custody or audio.
export const keptFlowExpression = String.raw`(async () => {
  const wait = async (predicate, name) => {
    const until = performance.now() + 10000;
    while (!predicate()) {
      if (performance.now() >= until) throw new Error('kept flow timed out: ' + name);
      await new Promise(resolve => setTimeout(resolve, 20));
    }
  };
  const find = (scope, text) => [...document.querySelectorAll(scope)].find(button => button.textContent.trim() === text);
  const completed = { cid: 'a'.repeat(64), manifest_version: 'c'.repeat(64), checked: false };
  const partial = { cid: 'b'.repeat(64), manifest_version: 'd'.repeat(64), transfer: 'e'.repeat(64), phase: 'paused', verified_chunks: 1, total_chunks: 3, running: false, user_paused: false, blocked: false, exhausted: false, grant: '1', intent: '1' };
  const saved = { ...partial, ...completed, transfer: 'f'.repeat(64), phase: 'completed', verified_chunks: 3, exhausted: true, grant: '9007199254740993' };
  const calls = [], inventory = { supported: true, allocated_bytes: 500000, limit_bytes: 1073741824, files: [completed], transfers: [saved, partial], error: null };
  const base = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  let check = null, registeredLease = null;
  const requireIdentity = (args, cid) => {
    if (args.server !== 1 || args.cid !== cid) throw new Error('wrong server or file identity');
  };
  window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
    if (command === 'get_kept_files') return structuredClone(inventory);
    if (command === 'get_files') {
      const result = await base(command, args), template = result.files[0];
      return { ...result, files: [{ ...template, cid: completed.cid, name: 'Saved example.bin', path: '', mime: 'application/octet-stream' }, { ...template, cid: partial.cid, name: 'Partial example.bin', path: '', mime: 'application/octet-stream' }] };
    }
    if (command === 'set_kept_paused') {
      calls.push([command, args]);
      requireIdentity(args, partial.cid);
      const target = inventory.transfers.find(copy => copy.cid === args.cid && copy.transfer === args.transfer);
      if (!target || target.intent !== args.expectedIntent) throw new Error('stale intent');
      target.intent = String(BigInt(target.intent) + 1n); target.user_paused = args.paused; target.running = false;
      return;
    }
    if (command === 'request_kept_file') { requireIdentity(args, partial.cid); if (partial.user_paused) throw new Error('Resume not committed'); calls.push([command, args]); partial.running = true; return; }
    if (command === 'retry_kept_file') {
      calls.push([command, args]);
      requireIdentity(args, saved.cid);
      if (!/^[0-9a-f]{32}$/.test(args.request)) throw new Error('invalid Retry identity');
      if (args.transfer !== saved.transfer || args.expectedGrant !== saved.grant) throw new Error('stale grant');
      saved.grant = String(BigInt(saved.grant) + 1n); saved.exhausted = false;
      return saved.grant;
    }
    if (command === 'begin_inline_download') { if (typeof args.cancellation !== 'string' || !args.cancellation.startsWith('keep-check-') || registeredLease) throw new Error('invalid check lease'); registeredLease = args.cancellation; calls.push([command, args]); return; }
    if (command === 'keep_file') { requireIdentity(args, saved.cid); if (!registeredLease || args.cancellation !== registeredLease) throw new Error('wrong check lease'); calls.push([command, args]); return new Promise((resolve, reject) => { check = { ...args, reject }; }); }
    if (command === 'cancel_inline_download') {
      if (!registeredLease || args.cancellation !== registeredLease) throw new Error('wrong cancellation lease');
      calls.push([command, args]);
      if (check?.cancellation === args.cancellation) { check.reject(new Error('check stopped')); check = null; }
      return;
    }
    if (command === 'forget_kept_file') { calls.push([command, args]); throw new Error('flow must not release ownership'); }
    return base(command, args);
  };
  const filesTab = [...document.querySelectorAll('.surface-bar button')].find(button => [...button.childNodes].some(node => node.nodeType === Node.TEXT_NODE && node.textContent.trim() === 'files'));
  if (!filesTab) throw new Error('files tab not found');
  filesTab.click();
  await wait(() => document.querySelector('.kept-copies-panel'), 'copy manager');
  document.querySelector('.kept-copies-panel').open = true;
  const panel = () => document.querySelector('.kept-copies-panel');
  const row = name => [...panel().querySelectorAll('.kept-copy-row')].find(row => row.textContent.includes(name));
  const action = (name, text) => [...row(name).querySelectorAll('button')].find(button => button.textContent.trim() === text);
  const clickAction = async (name, label) => {
    await wait(() => action(name, label) && !action(name, label).disabled, label + ' enabled');
    action(name, label).click();
  };
  const out = { quotaCountsOwnersOnce: panel().querySelector('summary').textContent.includes('2/32') };
  out.completedExhaustionOffersRetry = !!action('Saved example.bin', 'Retry check');
  await clickAction('Partial example.bin', 'Pause');
  await wait(() => partial.user_paused && row('Partial example.bin').textContent.includes('Paused'), 'persistent Pause');
  out.pauseKeepsGrant = partial.grant === '1';
  await clickAction('Partial example.bin', 'Resume');
  await wait(() => partial.running && row('Partial example.bin').textContent.includes('Saving'), 'Resume admission');
  out.resumeUsesNativeAdmission = calls.some(([command]) => command === 'request_kept_file');
  await clickAction('Partial example.bin', 'Pause');
  await wait(() => partial.user_paused && row('Partial example.bin').textContent.includes('Paused'), 'Pause running transfer');
  await clickAction('Saved example.bin', 'Retry check');
  await wait(() => check && find('.kept-copies-panel button', 'Stop checking'), 'cancelable completed repair');
  const lease = check.cancellation;
  find('.kept-copies-panel button', 'Stop checking').click();
  await wait(() => !check && action('Saved example.bin', 'Check and repair') && !action('Saved example.bin', 'Check and repair').disabled, 'stopped check');
  out.completedRetryIsLossless = saved.grant === '9007199254740994' && calls.filter(([command]) => command === 'retry_kept_file').length === 1;
  out.stopPreservesCopy = inventory.files.length === 1 && !calls.some(([command]) => command === 'forget_kept_file') && calls.some(([command, args]) => command === 'cancel_inline_download' && args.cancellation === lease);
  return out;
})()`;
