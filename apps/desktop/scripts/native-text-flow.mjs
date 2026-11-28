// Browser-only acceptance of the actual App route and its Native Text fixture.
// These are local-save/renderer claims; the Rust suite owns native authority.
export const nativeTextFlowExpression = String.raw`(async () => {
  const wait = async (predicate, label) => {
    const deadline = performance.now() + 12000;
    while (!predicate()) {
      if (performance.now() >= deadline) throw new Error('native text flow timed out: ' + label);
      await new Promise(resolve => setTimeout(resolve, 20));
    }
  };
  const input = (element, value) => {
    const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(element, value);
    element.dispatchEvent(new Event('input', { bubbles: true }));
  };
  if (new URLSearchParams(location.search).get('nativeText') === 'empty') {
    const entry = document.querySelector('[aria-label="Find or create a group"]');
    const localEntry = !!entry && !document.querySelector('#start-tab-join') && !!entry.querySelector('textarea');
    const calls = [], base = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
    window.__TAURI_INTERNALS__.invoke = async (command, args, options) => { calls.push(command); return base(command, args, options); };
    const fields = entry.querySelectorAll('input'); input(fields[0], 'Rowan'); input(fields[1], 'New local group');
    entry.querySelector('button').click();
    await wait(() => document.querySelector('.native-saved-chat'), 'created group selected');
    await wait(() => !document.querySelector('.native-saved-rows[aria-busy="true"]'), 'created history loaded');
    return { localEntry, created: calls.includes('found_server'), ownHistory: calls.includes('get_message_page'),
      noLegacy: !calls.some(name => ['preview_invite','join_server','get_channels','get_members','durable_send_context'].includes(name)),
      noUnavailableControls: !document.querySelector('.dm-circle,.inbox-circle,.orbit-btn,.join-channel,[title="Server settings"]') };
  }
  await wait(() => document.querySelectorAll('.native-saved-rows > article').length === 2, 'own saved originals');
  const root = document.querySelector('.native-saved-chat');
  const localScope = [...root.querySelectorAll('.native-saved-rows > article')]
    .every(row => row.querySelector('small')?.textContent.startsWith('You'));
  const calls = [], base = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  window.__TAURI_INTERNALS__.invoke = async (command, args, options) => { calls.push({command,args}); return base(command,args,options); };
  const composer = root.querySelector('textarea'); input(composer, 'Browser Native Text save');
  root.querySelector('form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await wait(() => [...root.querySelectorAll('article')].some(row => row.textContent.includes('Browser Native Text save')), 'new original rendered');
  const send = calls.find(call => call.command === 'send_message');
  const sendIndex = calls.findIndex(call => call === send);
  const sealedFirst = sendIndex >= 0 && calls.slice(0, sendIndex).some(call => {
    if (call.command !== 'save_ui_state') return false;
    try {
      const intent = JSON.parse(call.args.json).nativeTextIntents?.[send.args.retryToken];
      return intent?.kind === 'create' && intent.token === send.args.retryToken && intent.server === send.args.server
        && intent.channel === send.args.channel && intent.expectedContext === send.args.expectedContext && intent.text === send.args.text;
    } catch { return false; }
  });
  const members = document.querySelector('button[aria-controls="native-member-list"]');
  if (members.getAttribute('aria-expanded') === 'false') members.click();
  const namedButton = (scope, label) => [...(scope?.querySelectorAll('button') ?? [])]
    .find(button => button.textContent.trim() === label);
  const memberList = () => document.querySelector('#native-member-list');
  await wait(() => namedButton(memberList(), 'Chat history'), 'owner audit entry');
  namedButton(memberList(), 'Chat history').click();
  const audit = () => document.querySelector('dialog[open][aria-labelledby="native-audit-title"]');
  await wait(() => namedButton(audit(), 'View changes'), 'saved history index');
  namedButton(audit(), 'View changes').click();
  await wait(() => audit()?.textContent.includes('A retained edit from the same device.'), 'saved revision');
  const ownerAudit = audit().textContent.includes('Signed record details')
    && calls.filter(call => call.command === 'get_message_page' && 'auditAction' in call.args).length === 2;
  namedButton(audit(), 'Held posts').click();
  await wait(() => audit()?.textContent.includes('A late post held for review.'), 'held post audit');
  const heldAudit = calls.some(call => call.command === 'get_message_page'
    && call.args.heldAfter === null && call.args.limit === 2)
    && audit().textContent.includes('Awaiting review')
    && !audit().textContent.includes('A retained edit from the same device.');
  namedButton(audit(), 'Close').click();
  return { localScope, saved: !!send && /^[a-f0-9]{64}$/.test(send.args.retryToken),
    sealedFirst,
    composerRetired: composer.value === '', noLegacy: !calls.some(call => call.command === 'durable_send_context'),
    ownerAudit, heldAudit,
    noUnavailableControls: !document.querySelector('.dm-circle,.inbox-circle,.orbit-btn,.join-channel,[title="Server settings"]'),
    noOverflow: document.documentElement.scrollWidth <= innerWidth };
})()`;
