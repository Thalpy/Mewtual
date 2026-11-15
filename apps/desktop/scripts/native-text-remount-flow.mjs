// Actual Svelte unmount/mount, one unchanged simulated native IPC store.
// Native credentials/durability remain Rust's responsibility.
export const nativeTextRemountFlowExpression=String.raw`(async()=>{
  const wait=async(predicate,label)=>{const end=performance.now()+12000;while(!predicate()){if(performance.now()>end)throw new Error('remount flow timed out: '+label);await new Promise(resolve=>setTimeout(resolve,20));}};
  const input=(element,value)=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(element,value);element.dispatchEvent(new Event('input',{bubbles:true}));};
  const {replaceNativeFixtureApp}=await import('/scripts/native-text-remount-fixture.ts');
  const base=window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  const actualSet=window.setTimeout.bind(window),actualClear=window.clearTimeout.bind(window),timers=new Map();let nextTimer=1000000000;
  window.setTimeout=(callback,delay,...args)=>{if(delay===250||delay===5000){const id=nextTimer++;timers.set(id,{callback:()=>callback(...args),delay});return id;}return actualSet(callback,delay,...args);};
  window.clearTimeout=id=>{if(!timers.delete(id))actualClear(id);};
  let release=null,releaseNew=null,token=null,newToken=null,holds=true,writes=0,latestSaved=null;const forbidden=[];
  const spy=async(command,args={},options)=>{
    if(['lock_session','unlock','close_vault_window'].includes(command))forbidden.push(command);
    if(command==='save_ui_state')writes++;
    const value=await base(command,args,options);
    if(command==='save_ui_state')latestSaved=JSON.parse(args.json);
    if(command==='send_message'&&holds){holds=false;token=args.retryToken;return await new Promise(resolve=>{release=()=>resolve(value);});}
    if(command==='send_message'&&args.retryToken!==token){newToken=args.retryToken;return await new Promise(resolve=>{releaseNew=()=>resolve(value);});}
    return value;
  };
  window.__TAURI_INTERNALS__.invoke=spy;
  try{
    const oldComposer=document.querySelector('.native-saved-chat textarea');input(oldComposer,'Old view pending response');
    const debounce=[...timers.values()].find(timer=>timer.delay===250)?.callback;
    if(!debounce)throw new Error('old composer did not schedule its actual debounce');
    document.querySelector('.native-saved-chat form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));
    await wait(()=>release!==null,'old native receipt held');
    await replaceNativeFixtureApp();
    if(window.__TAURI_INTERNALS__.invoke!==spy)throw new Error('remount replaced the shared IPC fixture');
    await wait(()=>document.documentElement.dataset.visualReady==='chat'&&document.querySelector('.native-saved-chat textarea'),'new App ready on unchanged native session');
    await wait(()=>!document.querySelector('.native-saved-chat header button').disabled,'new App exact retry finished');
    const composer=document.querySelector('.native-saved-chat textarea');input(composer,'New view draft survives');
    const currentTimers=[...timers.values()].filter(timer=>timer.delay===250);if(!currentTimers.length)throw new Error('new debounce missing');currentTimers.forEach(timer=>timer.callback());
    try{await wait(()=>latestSaved?.drafts?.['1:1']==='New view draft survives','new draft sealed');}
    catch(error){throw new Error(error.message+' '+JSON.stringify({latestSaved,writes,token,snapshot:await base('get_ui_state',{})}));}
    document.querySelector('.native-saved-chat form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));
    await wait(()=>releaseNew!==null,'new component intent sealed before held response');
    const before=await base('get_ui_state',{}),count=writes;
    release();debounce();
    // Drain actual promise continuations, rather than wait for an arbitrary timer.
    for(let i=0;i<30;i++)await Promise.resolve();
    const after=await base('get_ui_state',{}),state=JSON.parse(after);
    return{warmSession:forbidden.length===0,oldCleanupDidNotSeal:writes===count,journalUnchanged:after===before,
      newDraftRetained:state.drafts?.['1:1']==='New view draft survives',newIntentRetained:!!state.nativeTextIntents?.[newToken]};
  }finally{window.setTimeout=actualSet;window.clearTimeout=actualClear;window.__TAURI_INTERNALS__.invoke=base;}
})()`;
