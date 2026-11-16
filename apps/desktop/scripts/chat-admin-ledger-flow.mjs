// Drive the actual App through its localhost fixture. Native replies are intercepted only to
// hold one real invocation across a server switch; no component or App state is injected.
// This proves renderer wiring/lifetime, not native authorization or cryptographic verification.
export const chatAdminLedgerFlowExpression = String.raw`(async () => {
  const wait = async (predicate, label) => {
    const deadline = performance.now() + 12000;
    while (!predicate()) {
      if (performance.now() >= deadline) throw new Error('chat ledger flow timed out: ' + label);
      await new Promise(resolve => setTimeout(resolve, 20));
    }
  };
  const find = (selector, text) => [...document.querySelectorAll(selector)].find(node => node.textContent.trim() === text);
  const base = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  const calls = [];
  let hold = false, held = null;
  window.__TAURI_INTERNALS__.invoke = async (command, args = {}, options) => {
    if (command === 'get_admin_chat_ledger' || command === 'get_admin_chat_ledger_detail') {
      calls.push({command, args});
      const reply = await base(command, args, options);
      if (command === 'get_admin_chat_ledger_detail' && hold) {
        hold = false;
        return await new Promise(resolve => { held = {resolve, reply}; });
      }
      return reply;
    }
    return await base(command, args, options);
  };
  const openHistory = async () => {
    await wait(() => document.querySelector('button.sidebar-mod'), 'moderation navigation');
    document.querySelector('button.sidebar-mod').click();
    await wait(() => find('.history-admin-tabs button', 'Chat history'), 'history tab');
    find('.history-admin-tabs button', 'Chat history').click();
    await wait(() => document.querySelectorAll('.history-record').length === 3, 'three retained originals');
  };
  const open = async index => {
    const buttons = [...document.querySelectorAll('.history-record')];
    await wait(() => buttons[index] && !buttons[index].disabled, 'original enabled');
    buttons[index].click();
    // A previous detail can still be in the DOM until Svelte flushes the click. Wait for this
    // exact row's successful selection AND re-enabled controls, not any pre-existing detail.
    await wait(() => buttons[index].getAttribute('aria-pressed') === 'true' && !buttons[index].disabled
      && document.querySelector('.history-detail') && !document.querySelector('.chat-admin-ledger [role="status"]'), 'selected original detail');
  };
  await openHistory();
  const out = { incompleteShown: document.querySelector('.chat-admin-ledger').textContent.includes('Some records or author details may be missing.') };
  await open(0);
  out.originalVisible = document.querySelector('.original-value')?.textContent === 'A first draft, before the edit.';
  await open(1);
  out.editVisible = document.querySelector('.original-value')?.textContent === 'The revised message.';
  await open(2);
  out.deleteVisible = document.querySelector('.history-detail')?.textContent.includes('Deleted')
    && !document.querySelector('.history-detail .original-value');
  // Finish with a completely valid, positive old native response after the component's actual
  // parent has switched to a different server. It must never land in the remounted admin pane.
  hold = true;
  document.querySelector('.history-record').click();
  await wait(() => held !== null, 'held original invocation');
  document.querySelector('button.dm-circle').click();
  await wait(() => document.querySelector('.dm-list button'), 'DM rail');
  document.querySelector('.dm-list button').click();
  await wait(() => !document.querySelector('.chat-admin-ledger') && document.querySelector('.server-title') === null, 'different server');
  const retired = held;
  retired.reply.legacyChange.mutations[0].value.text = 'PRIVATE OLD SCOPE MUST NOT LAND';
  retired.resolve(retired.reply); held = null;
  document.querySelector('.rail-scroll button[title="Lantern Room"]').click();
  await openHistory();
  out.oldScopeDiscarded = !document.body.textContent.includes('PRIVATE OLD SCOPE MUST NOT LAND')
    && !document.querySelector('.history-detail');
  out.exactCommands = calls.filter(call => call.command === 'get_admin_chat_ledger_detail')
    .every(call => call.args.server === 1 && /^legacy:1:[0-9a-f]{64}$/.test(call.args.rowId)
      && call.args.expectedRevision === '88'.repeat(16) && call.args.frameOffset === 0);
  await open(0);
  out.reopened = document.querySelector('.original-value')?.textContent === 'A first draft, before the edit.';
  document.querySelector('.history-detail').scrollIntoView({block: 'end'});
  return out;
})();`;
