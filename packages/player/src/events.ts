// A small typed event emitter for the player's events.
import type { Motion, Position } from './machine';

export interface PlayerEvents {
  /** The position changed; `motion` is how the stage reaches it. */
  position: { readonly position: Position; readonly previous: Position; readonly motion: Motion };
  /** A slide is on screen; `timedOut` when the readiness gate's cap ran out first. */
  ready: { readonly slide: number; readonly timedOut: boolean };
  /** `next` at the end of the show. */
  ended: Record<string, never>;
  diagnostic: { readonly code: string; readonly message: string; readonly slide: string | null };
  mediaerror: { readonly slide: string; readonly src: string };
  /** The presenter window opened (`null` when the browser blocked it). */
  presenter: { readonly window: Window | null };
}

export type Listener<K extends keyof PlayerEvents> = (event: PlayerEvents[K]) => void;

export class Emitter {
  #listeners = new Map<keyof PlayerEvents, Set<(event: never) => void>>();

  on<K extends keyof PlayerEvents>(type: K, listener: Listener<K>): () => void {
    let set = this.#listeners.get(type);
    if (set === undefined) {
      set = new Set();
      this.#listeners.set(type, set);
    }
    set.add(listener);
    return () => set.delete(listener);
  }

  emit<K extends keyof PlayerEvents>(type: K, event: PlayerEvents[K]): void {
    for (const listener of Array.from(this.#listeners.get(type) ?? [])) {
      try {
        (listener as Listener<K>)(event);
      } catch (error) {
        setTimeout(() => {
          throw error;
        });
      }
    }
  }

  clear(): void {
    this.#listeners.clear();
  }
}
