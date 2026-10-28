// Real rendered Transfers controls at the fixture IPC boundary. Native persistence/network
// acceptance lives in six_client_recovery/files.rs; this scenario proves their UI wiring.
export const privateDownloadFlowExpression = String.raw`(async () => {
  const wait = async (predicate, label) => {
    const until = performance.now() + 10000;
    while (!predicate()) {
      if (performance.now() >= until) throw new Error('download UI timed out: ' + label);
      await new Promise(resolve => setTimeout(resolve, 20));
    }
  };
  const base = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  const first = { id: 'd'.repeat(64), cid: 'a'.repeat(64), intent: '3', grant: '1', status: 'incomplete', verifiedChunks: 1, savedChunks: 0, totalChunks: 3, chosenPlace: false };
  const exhausted = { ...first, id: 'e'.repeat(64), cid: 'b'.repeat(64), status: 'exhausted', grant: '9007199254740993' };
  const ready = { ...first, id: 'f'.repeat(64), cid: 'c'.repeat(64), status: 'ready', verifiedChunks: 3 };
  // A journaled save whose partial file was replaced: it must offer Save new copy, never Resume.
  const changed = { ...first, id: '1'.repeat(64), cid: '2'.repeat(64), status: 'destination-changed', verifiedChunks: 3, savedChunks: 1 };
  const names = new Map([[first.id,'Interrupted drawing.bin'],[exhausted.id,'Large download.bin'],[ready.id,'Ready example.bin'],[changed.id,'Changed example.bin']]);
  let inventory = [first, exhausted, ready, changed], saving = null;
  const calls = [];
  const notify = () => base('plugin:event|emit', { event: 'files-updated', payload: { server: 1 } });
  window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
    if (command === 'get_private_downloads') return structuredClone(inventory);
    if (command === 'get_files') {
      const list = await base(command,args), template = list.files[0];
      return { ...list, files: inventory.map(row => ({ ...template, cid: row.cid,
        name: names.get(row.id), path: '', mime: 'application/octet-stream' })) };
    }
    if (command === 'retry_private_download') {
      if (args.download !== exhausted.id || args.expectedGrant !== exhausted.grant || !/^[0-9a-f]{32}$/.test(args.request)) throw new Error('invalid exact retry');
      calls.push([command,args]); exhausted.grant = String(BigInt(exhausted.grant) + 1n); exhausted.status = 'incomplete';
      return exhausted.grant;
    }
    if (command === 'save_group_file') {
      const record = inventory.find(row => row.id === args.resume && row.cid === args.cid);
      if (!record || args.expectedIntent !== record.intent || args.server !== 1 || !args.operation?.startsWith('save-') || saving) throw new Error('duplicate/stale resume');
      calls.push([command,args]); record.intent = String(BigInt(record.intent)+1n); record.status = 'receiving';
      void notify();
      return new Promise((resolve,reject) => { saving = { record, resolve, reject }; });
    }
    if (command === 'set_private_download_paused') {
      const record = inventory.find(row => row.id === args.download);
      if (!record || args.expectedIntent !== record.intent || args.paused !== true) throw new Error('stale pause');
      calls.push([command,args]); record.intent = String(BigInt(record.intent)+1n); record.status = 'paused';
      const old = saving; saving = null; old.reject(new Error('Download paused.')); void notify(); return;
    }
    if (command === 'cancel_private_download') throw new Error('flow must preserve saved progress');
    return base(command,args);
  };
  const tab = [...document.querySelectorAll('.surface-bar button')].find(b => [...b.childNodes]
    .some(node => node.nodeType === Node.TEXT_NODE && node.textContent.trim() === 'transfers'));
  if (!tab) throw new Error('Transfers tab missing'); tab.scrollIntoView({ block: 'nearest', inline: 'nearest' }); tab.click();
  // Transfers UX (PR #51): private downloads are .sdl-row cards (one row per file, carrying a
  // save's live progress); the live list below keeps .dl-item rows. Disabled row actions use
  // aria-disabled so their reason stays readable. Compact rows: a row's actions are icon buttons
  // whose accessible name is the action plus the file name ("Resume download drawing.bin"), so
  // they are found by that name; text buttons (the live list) still by their text.
  const rows = () => [...document.querySelectorAll('.downloads-tab .sdl-row, .downloads-tab .dl-item')];
  const row = name => rows().find(r => r.textContent.includes(name));
  const action = (name,label) => [...(row(name)?.querySelectorAll('button') ?? [])]
    .find(b => b.textContent.trim() === label || b.getAttribute('aria-label') === `${label} ${name}`);
  const off = button => !button || button.disabled || button.getAttribute('aria-disabled') === 'true';
  const click = async (name,label) => { await wait(() => action(name,label) && !off(action(name,label)),label); action(name,label).click(); };
  await wait(() => row('Interrupted drawing.bin'),'saved progress after opening Transfers');
  const out = { disclosure: document.querySelector('.downloads-tab').textContent.includes('Save new copy starts a separate file') };
  await wait(() => row('Changed example.bin'),'changed destination row');
  if (!action('Changed example.bin','Save new copy') || action('Changed example.bin','Resume saving') || action('Changed example.bin','Try again')) {
    throw new Error('a changed destination must offer Save new copy and never resume into the replaced file');
  }
  await click('Interrupted drawing.bin','Resume download');
  await wait(() => saving && first.status === 'receiving' && action('Interrupted drawing.bin','Pause'),'active metadata before save reply');
  // While receiving, Resume is not offered at all (absent), so a duplicate save cannot start.
  const duplicate = action('Interrupted drawing.bin','Resume download');
  out.duplicateDisabled = off(duplicate);
  out.pauseAvailable = !off(action('Interrupted drawing.bin','Pause'));
  await click('Interrupted drawing.bin','Pause');
  await wait(() => !saving && row('Interrupted drawing.bin').textContent.includes('Paused'),'paused encrypted progress');
  out.pausePreservesGrant = first.grant === '1' && first.verifiedChunks === 1;
  await click('Interrupted drawing.bin','Resume download');
  await wait(() => saving && first.status === 'receiving','new explicit resume');
  const done = saving; saving = null; inventory = inventory.filter(row => row !== first);
  done.resolve({ path: 'C:/Downloads/Interrupted drawing.bin', displayed: true }); await notify();
  await wait(() => rows().some(row => row.textContent.includes('Saved')),'resumed row settles');
  out.resumedSettles = rows().some(row => row.textContent.includes('Saved'));
  await click('Large download.bin','Retry download');
  await wait(() => saving?.record === exhausted,'exhausted explicit retry and resume');
  out.retryExact = exhausted.grant === '9007199254740994' && calls.filter(([name]) => name === 'retry_private_download').length === 1;
  // Leave the second acquisition pending so the screenshot includes usable Pause and an
  // independent completed copy. These are fixture-only promises, never actual disk writes.
  return out;
})()`;
