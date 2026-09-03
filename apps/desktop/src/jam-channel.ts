const CHANNEL_BRAND = Symbol("mewtual-jam-source-channel");

/**
 * Opaque capability for one exact authenticated data-channel generation.
 *
 * Event callbacks close over this object. Identity, not a caller-supplied generation number,
 * prevents an already queued callback from an older/removed channel being mistaken for current.
 */
export type JamSourceChannel = Readonly<{
  source: string;
  serial: number;
  [CHANNEL_BRAND]: true;
}>;

export class JamSourceChannelRegistry {
  private readonly currentBySource = new Map<string, JamSourceChannel>();
  private nextSerial = 1;

  open(source: string): JamSourceChannel {
    if (!source) throw new TypeError("jam source channel needs an authenticated source identity");
    const token = Object.freeze({ source, serial: this.nextSerial, [CHANNEL_BRAND]: true as const });
    this.nextSerial = this.nextSerial >= Number.MAX_SAFE_INTEGER ? 1 : this.nextSerial + 1;
    this.currentBySource.set(source, token);
    return token;
  }

  isCurrent(channel: JamSourceChannel): boolean {
    return !!channel && this.currentBySource.get(channel.source) === channel;
  }

  current(source: string): JamSourceChannel | null {
    return this.currentBySource.get(source) ?? null;
  }

  close(source: string): boolean {
    return this.currentBySource.delete(source);
  }

  clear(): void {
    this.currentBySource.clear();
  }
}
