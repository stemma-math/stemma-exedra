// A question before an action, in the app's own dialog: the browser's is not
// Stemma's, and the desktop app does not show it.

export interface Question {
  title: string;
  text?: string;
  /** What the confirming button says. */
  action: string;
  /** What the button that goes back says. */
  dismiss?: string;
  /** Whether the action loses something. */
  danger?: boolean;
}

export const asking = $state<{
  question: Question | null;
  answer: ((yes: boolean) => void) | null;
}>({
  question: null,
  answer: null,
});

/** Asks before an action; resolves to whether the person went ahead. */
export function ask(question: Question): Promise<boolean> {
  asking.answer?.(false);
  return new Promise((resolve) => {
    asking.question = question;
    asking.answer = resolve;
  });
}
