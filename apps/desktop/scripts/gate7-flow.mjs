// Real App.svelte button/DOM/canvas flow through the explicit localhost fixture. Native file
// custody/call transport are mocked here; separate Rust acceptance exercises their actual bodies.
export function gate7FlowExpression(golden) {
  return `(async () => {
    const notices = new Set();
    new MutationObserver(() => document.querySelectorAll('.toast-text,.error-toast').forEach(e => notices.add(e.textContent))).observe(document.body, { childList: true, subtree: true });
    const wait = async (predicate, label) => { const end = performance.now() + 12000; while (performance.now() < end) { if (predicate()) return; await new Promise(r => setTimeout(r, 25)); } throw new Error('timed out: ' + label + ' · notices=' + JSON.stringify([...notices]) + ' · ' + document.body.innerText.slice(-3000) + ' · commands=' + JSON.stringify(window.__gate7Commands)); };
    const button = (selector, text) => [...document.querySelectorAll(selector)].find(e => e.textContent.trim().includes(text));
    const capture = name => new Promise(resolve => { window.__gate7Captured = resolve; window.gate7Capture(name); });
    const type = (element, text) => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(element, text); element.dispatchEvent(new Event('input', { bubbles: true })); };
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
    const reads = [], cancels = [], sent = [], uploaded = [];
    let deferUpload = false, releaseUpload = null, closingSnapshot = null;
    window.__gate7Commands = [];
    window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      window.__gate7Commands.push(command);
      if (command === 'get_files') { const list = await base(command, args); return { ...list, files: [file, ...list.files] }; }
      if (command === 'download_file') { reads.push(args); if (args.cid === cid) return encoded; }
      if (command === 'finish_file_upload') {
        if (deferUpload) await new Promise(resolve => { releaseUpload = resolve; });
        const result = await base(command, args); uploaded.push(result); return result;
      }
      // The close regression holds the native close after it captures continuity. It never
      // locks/destroys a real window; a late upload still must lose composer authority.
      if (command === 'close_vault_window') { closingSnapshot = args.uiStateJson; return new Promise(() => {}); }
      if (command === 'durable_send_context') return 'a'.repeat(64);
      if (command === 'send_message') { sent.push({ id: 'gate7-sent-' + sent.length, author: file.author, text: args.text, ts: Date.now(), edited: 0, reactions: [], reply_to: '', pinned: false }); return { accepted: true, persistence: {status: 'durable'} }; }
      if (command === 'get_message_page' && args.server === 1 && args.channel === '1') { const page = await base(command, args); return { ...page, rows: [...page.rows, ...sent.map(m => ({ ...m, targets_me: false, reply_count: 0, reply_to_preview: null }))], total: page.total + sent.length }; }
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
    await wait(() => document.querySelector('.file-info .pixa-preview canvas:not([hidden])'), 'inline file preview');
    const inlineFile = document.querySelector('.file-info .pixa-preview canvas');
    await wait(() => Number(inlineFile.dataset.pixaFrame) >= 1, 'inline file playback');
    const out = { fileInlinePlays: true };
    await wait(() => inlineFile.dataset.pixaFrame === '0', 'inline file loop'); out.fileInlineLoops = true;
    await capture('file-preview');
    await wait(() => button('.file-info-actions button', 'Open flipnote'), 'global viewer entry'); button('.file-info-actions button', 'Open flipnote').click();
    await wait(() => document.querySelector('[aria-label="Flipnote viewer"] canvas'), 'decoded viewer canvas');
    const globalCanvas = document.querySelector('[aria-label="Flipnote viewer"] canvas');
    const redAt = (canvas, x, y) => canvas.getContext('2d').getImageData(x,y,1,1).data[0];
    out.viewerFirstFrame = globalCanvas.dataset.pixaFrame === '0'; out.viewerHasPixels = redAt(globalCanvas, 32, 64) === 249;
    // The viewer's transport has several icon buttons whose hidden labels mention playing; the
    // play/pause control is the one whose text is exactly "Play" while stopped.
    await click([...document.querySelectorAll('[aria-label="Flipnote viewer"] button')].find(b => b.textContent.trim() === 'Play'));
    await wait(() => Number(globalCanvas.dataset.pixaFrame) >= 1, 'viewer playback'); out.viewerAdvances = true;
    out.viewerRepaints = redAt(globalCanvas, 32, 64) === 24;
    document.querySelector('[aria-label="Close flipnote viewer"]').click();
    await wait(() => !document.querySelector('[aria-label="Flipnote viewer"]'), 'viewer close');
    out.viewerClosed = !document.querySelector('[aria-label="Flipnote viewer"]');
    // Export the actual fixture composition through the real upload loop, then Send through
    // the ordinary durable composer. No fabricated Studio reference/owner receipt is involved.
    button('.surface-bar button', 'chat').click();
    await wait(() => document.querySelector('.composer textarea'), 'chat composer');
    type(document.querySelector('.composer textarea'), 'A flipnote for chat');
    button('.surface-bar button', 'studio').click();
    await wait(() => document.querySelector('.studio-obj'), 'Studio entries'); document.querySelector('.studio-obj').click();
    await wait(() => button('.st-btn', 'Post to chat') && document.querySelector('.st-well'), 'ready editor');
    out.studioTabs = [...document.querySelectorAll('.st-itab')].map(e => e.textContent.trim()).join(',') === 'art,sound,music,history';
    const fitted = document.querySelector('.st-canvas-wrap').getBoundingClientRect();
    out.studioCanvasFits = fitted.width >= 192 && fitted.right <= 900 && fitted.height / fitted.width > .74;
    await capture('art');
    for (const tab of ['sound', 'music', 'history']) { button('.st-itab', tab).click(); await wait(() => button('.st-itab.on', tab), tab + ' tab'); await capture(tab); }
    button('.st-btn', 'Post to chat').click();
    await wait(() => document.querySelector('.composer textarea')?.value.includes('cid:'), 'Post to chat attachment');
    const composer = document.querySelector('.composer textarea'), attached = composer.value;
    out.postPreservesDraft = attached.startsWith('A flipnote for chat ') && attached.includes(uploaded[0]) && !attached.includes(cid);
    const stored = await base('download_file', {server: 1, cid: uploaded[0], maxBytes: 9 * 1024 * 1024});
    const published = decodePixa(Uint8Array.from(atob(stored), c => c.charCodeAt(0)));
    out.postUploadedPixa = published.frames.length === 3 && published.take.group === 'deadbeef';
    composer.closest('form').requestSubmit();
    await wait(() => document.querySelector('.messages .pixa-preview canvas:not([hidden])'), 'inline chat attachment');
    out.postSendsNormally = sent.length === 1 && sent[0].text === attached && composer.value === '';
    const inlineChat = document.querySelector('.messages .pixa-preview canvas');
    await wait(() => Number(inlineChat.dataset.pixaFrame) >= 1, 'inline chat playback'); out.chatInlinePlays = true;
    await wait(() => inlineChat.dataset.pixaFrame === '0', 'inline chat loop'); out.chatInlineLoops = true;
    await capture('chat');
    // A delayed upload may remain in Files, but returning to the editor must not revive its
    // permission to change another draft after navigation (including an A → B → A return).
    type(document.querySelector('.composer textarea'), 'Keep this draft');
    button('.surface-bar button', 'studio').click();
    await wait(() => button('.st-btn', 'Post to chat') && !button('.st-btn', 'Post to chat').disabled, 'second post entry');
    deferUpload = true; button('.st-btn', 'Post to chat').click();
    await wait(() => releaseUpload, 'delayed upload');
    button('.surface-bar button', 'chat').click(); await wait(() => document.querySelector('.composer textarea'), 'leave pending post');
    button('.surface-bar button', 'studio').click(); await wait(() => button('.st-btn', 'Post to chat'), 'return during pending post');
    releaseUpload(); await wait(() => uploaded.length === 2, 'delayed upload finished');
    await wait(() => button('.st-btn', 'Post to chat') && !button('.st-btn', 'Post to chat').disabled, 'stale post settled');
    button('.surface-bar button', 'chat').click(); await wait(() => document.querySelector('.composer textarea'), 'preserved draft');
    out.stalePostRevoked = document.querySelector('.composer textarea').value === 'Keep this draft' && sent.length === 1;
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
    // Close has already snapshotted the draft. Completing an upload now must not append a
    // message that the saved snapshot cannot contain, or report it as attached.
    document.querySelector('[aria-label="Leave focus"]')?.click();
    button('.surface-bar button', 'studio').click();
    await wait(() => button('.st-btn', 'Post to chat') && !button('.st-btn', 'Post to chat').disabled, 'post before close');
    releaseUpload = null; button('.st-btn', 'Post to chat').click(); await wait(() => releaseUpload, 'upload pending at close');
    await base('plugin:event|emit', {event: 'tauri://close-requested', payload: null});
    await wait(() => closingSnapshot !== null, 'close continuity captured');
    releaseUpload(); await wait(() => uploaded.length === 3, 'upload completed after close');
    await wait(() => button('.st-btn', 'Post to chat') && !button('.st-btn', 'Post to chat').disabled, 'closed post settled');
    const stayedInStudio = !!document.querySelector('.st-canvas-wrap');
    button('.surface-bar button', 'chat').click(); await wait(() => document.querySelector('.composer textarea'), 'draft after close');
    out.closePostRevoked = stayedInStudio && document.querySelector('.composer textarea').value === 'Keep this draft' && closingSnapshot.includes('Keep this draft') && !closingSnapshot.includes(uploaded[2]);
    return out;
  })()`;
}
