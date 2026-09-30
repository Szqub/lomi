/** xterm 6.0.0 reports keyboard/paste/IME/mouse input separately from parser replies. */
interface TerminalInput {
  onData(listener: (data: string) => void): { dispose(): void };
}
interface CoreData {
  triggerDataEvent(data: string, wasUserInput?: boolean): void;
}
export function classifyTerminalInput(
  terminal: TerminalInput,
  listener: (data: string, human: boolean) => void,
): { dispose(): void } {
  const service = (
    terminal as TerminalInput & { _core?: { coreService?: CoreData } }
  )._core?.coreService;
  if (!service || typeof service.triggerDataEvent !== "function")
    throw new Error("Unsupported terminal input classifier.");
  const original = service.triggerDataEvent;
  let human = false;
  function classified(this: CoreData, data: string, wasUserInput = false) {
    const previous = human;
    human = wasUserInput;
    try {
      original.call(this, data, wasUserInput);
    } finally {
      human = previous;
    }
  }
  service.triggerDataEvent = classified;
  const subscription = terminal.onData((data) => listener(data, human));
  return {
    dispose() {
      subscription.dispose();
      if (service.triggerDataEvent === classified)
        service.triggerDataEvent = original;
    },
  };
}
