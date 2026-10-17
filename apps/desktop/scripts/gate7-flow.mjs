// Real App.svelte button/DOM/canvas flow through the explicit localhost fixture. Native file
// custody/call transport are mocked here; separate Rust acceptance exercises their actual bodies.
export function gate7FlowExpression(golden) {
  return `(async () => {
    const wait = async (predicate, label) => { const end = performance.now() + 12000; while (performance.now() < end) { if (predicate()) return; await new Promise(r => setTimeout(r, 25)); } throw new Error('timed out: ' + label + ' · ' + document.body.innerText.slice(-3000) + ' · commands=' + JSON.stringify(window.__gate7Commands)); };
    const button = (selector, text) => [...document.querySelectorAll(selector)].find(e => e.textContent.trim().includes(text));
    const click = async (element) => {
      if (!element) throw new Error('missing gesture target');
      element.scrollIntoView({ block: 'center' });
      const r = element.getBoundingClientRect();
      await new Promise(resolve => { window.__gate7Clicked = resolve; window.gate7Click(JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 })); });
    };
    const { decodePixa, encodePixa } = await import('/src/pixa.ts');
    const { encodePix } = await import('/src/pix.ts');
    const { bytesToBase64 } = await import('/src/studio-native.ts');
    const original = decodePixa(Uint8Array.from(${JSON.stringify(golden.pixaHex)}.match(/../g), h => parseInt(h, 16)));
    // Distinct, known pixels prove actual canvas replacement, not only a changing frame label.
    const palette = [{ role: 0, r: 24, g: 18, b: 38 }, { role: 0, r: 249, g: 208, b: 88 }, { role: 0, r: 109, g: 222, b: 202 }, { role: 0, r: 174, g: 142, b: 245 }];
    const frames = [0, 1, 2].map(frame => {
      const pixels = new Uint8Array(192 * 144), left = 24 + frame * 56;
      for (let y = 52; y < 92; y++) for (let x = left; x < left + 32; x++) pixels[y * 192 + x] = frame + 1;
      return encodePix({ w: 192, h: 144, palette, pixels });
    });
    const packageValue = { ...original, fps: 2, frames, take: { ...original.take, patches: [${JSON.stringify(golden.patch)}] }, sfx: [{ frame: 1, patch: 0, note: 72, durationMs: 2000 }] };
    const bytes = encodePixa(packageValue), encoded = bytesToBase64(bytes), cid = 'e'.repeat(64);
    const file = { name: 'Gate-7-animation.pixa', mime: 'application/x-mewtual-pixa', cid, size: bytes.length, author: 'a4f29c110b7d8365a4f29c110b7d8365', author_identity: 'a4f29c110b7d8365a4f29c110b7d8365'.repeat(2), author_verified: true, path: '', held: 1, total: 1, expires: null, expires_known: true };
    const base = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
    const reads = [], cancels = [];
    window.__gate7Commands = [];
    window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      window.__gate7Commands.push(command);
      if (command === 'get_files') return { has_peers: false, files: [file] };
      if (command === 'download_file') { reads.push(args); return encoded; }
      if (command === 'begin_inline_download') return;
      if (command === 'cancel_inline_download') { cancels.push(args.cancellation); return; }
      if (command === 'get_online_members') return [];
      if (command === 'get_members') return [{ fingerprint: file.author, identity: file.author_identity, you: true }];
      if (command === 'get_jukebox') return [{ id: 'gate7', cid, name: file.name, author: file.author_identity, added_ms: 1 }];
      if (command === 'file_available') return true;
      return base(command, args);
    };
    // This fixture never opens a real microphone prompt or contacts another desktop.
    if (navigator.mediaDevices) navigator.mediaDevices.getUserMedia = async () => { throw new Error('fixture has no microphone'); };
    button('.surface-bar button', 'files').click();
    await wait(() => button('.file-name', file.name), 'PIXA file listing'); button('.file-name', file.name).click();
    await wait(() => button('.file-info-actions button', 'Open flipnote'), 'global viewer entry'); button('.file-info-actions button', 'Open flipnote').click();
    await wait(() => document.querySelector('[aria-label="Flipnote viewer"] canvas'), 'decoded viewer canvas');
    const globalCanvas = document.querySelector('[aria-label="Flipnote viewer"] canvas');
    const redAt = (canvas, x, y) => canvas.getContext('2d').getImageData(x,y,1,1).data[0];
    const out = { viewerFirstFrame: globalCanvas.dataset.pixaFrame === '0', viewerHasPixels: redAt(globalCanvas, 32, 64) === 249 };
    await click(button('[aria-label="Flipnote viewer"] button', 'Play from start'));
    await wait(() => Number(globalCanvas.dataset.pixaFrame) >= 1, 'viewer playback'); out.viewerAdvances = true;
    out.viewerRepaints = redAt(globalCanvas, 32, 64) === 24;
    document.querySelector('[aria-label="Close flipnote viewer"]').click();
    await wait(() => !document.querySelector('[aria-label="Flipnote viewer"]'), 'viewer close');
    out.viewerClosed = !document.querySelector('[aria-label="Flipnote viewer"]');
    button('.surface-bar button', 'chat').click();
    await wait(() => document.querySelector('.call-start'), 'voice join'); document.querySelector('.call-start').click();
    await wait(() => document.querySelector('[aria-label="Open the voice stage"]'), 'collapsed voice stage');
    await click(document.querySelector('[aria-label="Open the voice stage"]'));
    await wait(() => document.querySelector('.juke-nm'), 'shared queue'); await click(document.querySelector('.juke-nm'));
    await wait(() => document.querySelector('.juke-chip-btn')?.textContent.trim() === 'LOAD TRACK' || document.querySelector('.juke-screen canvas'), 'local media admission');
    if (button('.juke-chip-btn', 'LOAD TRACK')) await click(button('.juke-chip-btn', 'LOAD TRACK'));
    await wait(() => document.querySelector('.juke-screen canvas'), 'PIXA on deck');
    out.deckHasCanvas = true; out.noMediaElementForPixa = !document.querySelector('#jukebox-media[src]');
    document.querySelector('[aria-label="Watch full size"]').click();
    await wait(() => document.querySelector('.focus-deck canvas'), 'focus canvas'); out.focusHasCanvas = true;
    await wait(() => Number(document.querySelector('.focus-deck canvas')?.dataset.pixaFrame) >= 1, 'shared animation advances'); out.deckAdvances = true;
    out.deckRepaints = redAt(document.querySelector('.focus-deck canvas'), 32, 64) === 24;
    const pause = document.querySelector('.focus .juke-play[aria-label="Pause"]');
    if (!pause) throw new Error('no room pause control'); pause.click();
    await new Promise(r => setTimeout(r, 100));
    const held = document.querySelector('.focus-deck canvas')?.dataset.pixaFrame;
    await new Promise(r => setTimeout(r, 600));
    out.pauseHoldsFrame = !!held && document.querySelector('.focus-deck canvas')?.dataset.pixaFrame === held;
    out.nativeCaps = reads.length >= 2 && reads.every(r => r.maxBytes === 9 * 1024 * 1024 && typeof r.cancellation === 'string');
    out.permitsRetired = cancels.length >= 2;
    return out;
  })()`;
}
