import type { MappedPort } from "./callroutes.ts";

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type Grant = MappedPort & { grant: string };
type Join = { lease: number; owner: string | null; ready: Promise<string | null> };

/** Frontend custody for native controller/call capabilities. Native code owns router effects;
 * these fences only prevent a retired mount/join from activating or using their results. */
export class CallMappingSession {
  private epoch = 0;
  private destroyed = false;
  private controller: string | null = null;
  private claiming: Promise<string | null> | null = null;
  private join: Join | null = null;

  private readonly invoke: Invoke;
  constructor(invoke: Invoke) { this.invoke = invoke; }

  private end(owner: string): void {
    void this.invoke("end_call_mapping_owner", { owner }).catch(() => {});
  }

  /** Claim after unlock/resume, even if no call is joined: a remount must retire its predecessor. */
  claimController(): Promise<string | null> {
    if (this.destroyed) return Promise.resolve(null);
    if (this.controller) return Promise.resolve(this.controller);
    if (this.claiming) return this.claiming;
    const epoch = this.epoch;
    const current = () => !this.destroyed && this.epoch === epoch;
    const operation = (async () => {
      // A simultaneous old-controller End may advance the native revision. Retry only while
      // this exact mount is current; bounded local IPC retries never retry a router operation.
      for (let attempt = 0; attempt < 3 && current(); attempt++) {
        let candidate: string | null = null;
        try {
          candidate = await this.invoke("prepare_call_mapping_controller") as string;
          if (!current()) { this.end(candidate); return null; }
          const owner = await this.invoke("activate_call_mapping_owner", { candidate }) as string;
          if (!current()) { this.end(owner); return null; }
          this.controller = owner;
          return owner;
        } catch {
          if (candidate) this.end(candidate);
        }
      }
      return null;
    })();
    this.claiming = operation;
    void operation.finally(() => { if (this.claiming === operation) this.claiming = null; });
    return operation;
  }

  /** Begin one local join. A failed/late prepare can never end a different join's native owner. */
  start(lease: number, server: number, room: string): void {
    this.endCall();
    if (this.destroyed) return;
    const join: Join = { lease, owner: null, ready: Promise.resolve(null) };
    this.join = join;
    const current = () => !this.destroyed && this.join === join;
    join.ready = (async () => {
      const controller = await this.claimController();
      if (!controller || !current()) return null;
      for (let attempt = 0; attempt < 3 && current(); attempt++) {
        let candidate: string | null = null;
        try {
          candidate = await this.invoke("prepare_call_mapping_owner", { controller, server, room }) as string;
          if (!current()) { this.end(candidate); return null; }
          const owner = await this.invoke("activate_call_mapping_owner", { candidate }) as string;
          if (!current()) { this.end(owner); return null; }
          join.owner = owner;
          return owner;
        } catch {
          if (candidate) this.end(candidate);
        }
      }
      return null;
    })();
  }

  /** Map only for the exact join. Native code retains pending ownership through End/lock. */
  async map(lease: number, port: number, address: string | null): Promise<MappedPort | null> {
    const join = this.join;
    if (!join || join.lease !== lease) return null;
    const owner = await join.ready;
    if (!owner || this.destroyed || this.join !== join) return null;
    const grant = await this.invoke("map_call_port", { owner, port, address }) as Grant;
    if (this.destroyed || this.join !== join) {
      // End already revokes native pending work. Exact compensation also covers a result that
      // committed just before End; it cannot remove another call's same-port grant.
      void this.invoke("unmap_call_port", { owner, grant: grant.grant }).catch(() => {});
      return null;
    }
    return grant;
  }

  endCall(): void {
    const join = this.join;
    this.join = null;
    if (join?.owner) this.end(join.owner);
  }

  resetController(): void {
    this.epoch++;
    this.endCall();
    if (this.controller) this.end(this.controller);
    this.controller = null;
    this.claiming = null;
  }

  destroy(): void { this.destroyed = true; this.resetController(); }
}
