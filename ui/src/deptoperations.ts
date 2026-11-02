// A70: a UI observation deadline is not the lifetime of the native operation.
// Keep the original promise owned until settlement, even after the view says pending.
export type DeptOperationOutcome<T> = { ok: true; value: T } | { ok: false; error: unknown };
export type DeptOperationView<T> = { state: "done"; value: T } | { state: "pending" };

export type DeptOperationTarget = { kind: "base" } | { kind: "department"; socket?: string };

export class DepartmentOperations {
  private readonly active = new Map<string, { target?: DeptOperationTarget }>();
  constructor(
    private readonly schedule: (f: () => void, ms: number) => unknown = (f, ms) => setTimeout(f, ms),
    private readonly unschedule: (id: unknown) => void = (id) => clearTimeout(id as ReturnType<typeof setTimeout>),
  ) {}

  has(key: string): boolean { return this.active.has(key); }
  busy(): boolean { return this.active.size > 0; }
  blocksClose(socket: string | undefined, same: (a: string | undefined, b: string | undefined) => boolean): boolean {
    for (const { target } of this.active.values()) {
      if (!target) return true; // Unknown identity: closing is not provably independent.
      if (target.kind === "base") { if (socket == null) return true; continue; }
      if (socket == null) continue;
      if (!target.socket || same(target.socket, socket)) return true;
    }
    return false;
  }

  async observe<T>(
    key: string,
    launch: () => Promise<T>,
    viewMs: number,
    onLate: (result: DeptOperationOutcome<T>) => void,
    target?: DeptOperationTarget,
  ): Promise<DeptOperationView<T>> {
    if (this.active.has(key)) return { state: "pending" };
    const owner = { target };
    this.active.set(key, owner);
    let pending = false;
    let timer: unknown;
    // Turn synchronous invocation errors into the same owned settlement path.
    const completion: Promise<DeptOperationOutcome<T>> = Promise.resolve().then(launch).then(
      (value) => ({ ok: true, value }), (error) => ({ ok: false, error }),
    );
    const settled = completion.then((result) => {
      if (this.active.get(key) === owner) this.active.delete(key);
      this.unschedule(timer);
      if (pending) {
        try { onLate(result); } catch { /* presentation cannot reopen a native operation */ }
      }
      return result;
    });
    const expiry = new Promise<{ pending: true }>((resolve) => {
      timer = this.schedule(() => { pending = true; resolve({ pending: true }); }, viewMs);
    });
    const result = await Promise.race([settled, expiry]);
    if ("pending" in result) return { state: "pending" };
    if (!result.ok) throw result.error;
    return { state: "done", value: result.value };
  }
}
