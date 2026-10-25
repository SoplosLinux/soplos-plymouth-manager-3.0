import { writable } from "svelte/store";

// One dialog at a time, asked for from anywhere and answered with a promise:
// the window draws whichever is current (see components/DialogHost.svelte).
// Dialogs asked while another is open wait for their turn.

export type DialogKind = "info" | "error" | "confirm" | "danger";

export interface Dialog {
  kind: DialogKind;
  title: string;
  body: string;
  /** Lines of output shown under the text, e.g. what a failed command said. */
  details?: string;
  confirmLabel?: string;
  resolve: (answer: boolean) => void;
}

export const dialog = writable<Dialog | null>(null);

let queue: Promise<unknown> = Promise.resolve();

function open(spec: Omit<Dialog, "resolve">): Promise<boolean> {
  const turn = queue.then(
    () =>
      new Promise<boolean>((resolve) => {
        dialog.set({
          ...spec,
          resolve: (answer: boolean) => {
            dialog.set(null);
            resolve(answer);
          },
        });
      }),
  );
  queue = turn.catch(() => undefined);
  return turn;
}

export const showInfo = (title: string, body: string) => open({ kind: "info", title, body });
export const showError = (title: string, body: string, details?: string) => open({ kind: "error", title, body, details });
export const askConfirm = (title: string, body: string, confirmLabel?: string, danger = false) =>
  open({ kind: danger ? "danger" : "confirm", title, body, confirmLabel });
