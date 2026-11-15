// Owned browser-test control, never imported by the production entry point.
// Keep the existing IPC fixture/store: remounting must not reauthenticate it.
import {mount,unmount} from "svelte";
import App from "../src/App.svelte";
import first from "../src/main.ts";
let current:Awaited<typeof first>|null=null;
export async function replaceNativeFixtureApp(){
  const params=new URLSearchParams(location.search);
  if(!import.meta.env.DEV||location.hostname!=="localhost"||params.get("fixture")!=="chat"||!params.has("nativeText"))throw new Error("Native remount fixture unavailable");
  const original=current??await first;if(!original)throw new Error("Original App did not mount");
  await unmount(original);
  delete document.documentElement.dataset.visualReady;
  current=mount(App,{target:document.getElementById("app")!});
}
