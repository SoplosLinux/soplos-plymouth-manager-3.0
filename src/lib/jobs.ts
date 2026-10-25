import { derived, get, writable } from "svelte/store";
import { _ } from "svelte-i18n";
import { api } from "./api";

// A job is a command of the backend that takes a while (applying a theme,
// installing, removing, previewing). The command answers with an id at once;
// this follows it until it ends. While one runs, the bar along the bottom edge
// of the window moves.

interface JobStatus {
  running: boolean;
  success: boolean;
  output: string;
}

export interface JobResult {
  success: boolean;
  output: string;
}

const running = writable(0);

/** True while a job runs. */
export const busy = derived(running, (count) => count > 0);

const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export async function runJob(command: string, args?: Record<string, unknown>): Promise<JobResult> {
  running.update((count) => count + 1);
  try {
    const jobId = await api.invoke<number>(command, args);
    while (true) {
      const status = await api.invoke<JobStatus>("poll_job", { jobId });
      if (!status.running) return { success: status.success, output: status.output };
      await pause(300);
    }
  } catch (error) {
    return { success: false, output: String(error) };
  } finally {
    running.update((count) => count - 1);
  }
}

/**
 * The output of a job as a person reads it: the messages of the program (the
 * lines `@@step <name> <argument>`) in the language of the window, what a tool
 * printed as it is, and nothing of the program's own bookkeeping. Only the last
 * lines, which are the ones that say what went wrong.
 */
export function describeOutput(output: string): string {
  const t = get(_);
  const lines: string[] = [];
  for (const raw of output.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith("@@step ")) {
      const [key, ...argument] = line.slice("@@step ".length).split(" ");
      lines.push(t(`steps.${key}`, { values: { arg: argument.join(" ") } }));
    } else if (!line.startsWith("@@")) {
      lines.push(line);
    }
  }
  return lines.slice(-12).join("\n");
}

/** Whether the program sent that message during the job. */
export function hasStep(output: string, key: string): boolean {
  return output.split("\n").some((line) => line.trim() === `@@step ${key}` || line.trim().startsWith(`@@step ${key} `));
}
