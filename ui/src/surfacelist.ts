// ★F-01(0.14.52 · U · R3-4) 주기 `surface.list` 운반 — 끝점마다 **소유한 원 호출 하나**, 응답마다 **독립 투영**, 소비자용 **최신 미소비 칸 하나**.
//
// 왜 따로 두나: 종전 3초 틱은 소켓을 **직렬로 await** 했고(먹통 데몬 하나가 뒤 소켓들의 갱신을 매 틱 D 만큼 늦춘다) 범용 in-flight 가드는
// 60/120초가 지나면 아직 안 끝난 원 호출을 **대체**하고, 옛 호출이 끝날 때 **소유 확인 없이** 키를 지웠다(main.ts claimFlight). 이 모듈은
// 그 셋을 이 생산자에 한해 끊는다(다른 RPC 손님의 정책은 그대로 둔다):
//   · 원 호출은 실제로 끝날(settle) 때까지 그 끝점을 소유한다 — 시간 상한(D)·나이·워크스페이스 제거/재추가로 풀리지 않는다.
//     풀 때는 **그 원 호출 자신의 소유권만** 내려놓는다(다른 원 호출의 소유를 지우지 않는다).
//   · 프런트엔드 상한(D)은 '그 데이터는 소비자에게 없음'일 뿐 풀(pool) RPC 를 취소하지 않는다(취소하면 뒤 응답이 엉뚱한 요청에 붙는다 —
//     src-tauri/src/main.rs rpc_on 주석).
//   · 끝점 수명(lifetime)은 소유권과 **별개**다 — 로스터에서 빠지면 수명이 끝나고, 다시 들어오면 새 수명이다. 옛 수명의 응답은 투영·소비 모두 버린다.
//   · 기본(undefined)과 같은 경로를 가리키는 명시 소켓은 읽기 전용 Tauri `resolve_list_endpoints`(기존 resolve_socket)의 **그 로스터 판의 결과**로
//     한 끝점이 된다 — 문자열 Set 은 물리 동일성의 증거가 아니다. 판정은 로스터 판에 묶이고 비행 도중 다시 계산하지 않는다.
//     An entry that cannot be resolved or represented keeps its own endpoint (`raw:<socket>`) and is sent with its socket value as before;
//     it only loses alias merging. unresolved() lists such members for tests/diagnostics — nothing is shown or logged to the user.
// 타이머·재시도·새 주기 RPC 0 — 호출 시점은 전부 기존 3초 틱(main.ts refreshPaneTitles)이 정한다.

export interface ListArrival {
  /** 정규 끝점 키(해석된 명시 경로, 또는 미해석이면 `raw:<소켓>`). */
  endpoint: string;
  /** 그 원 호출을 보낸 때의 끝점 수명(전역 증가 번호). */
  lifetime: number;
  /** 이 끝점에 붙은 로스터 구성원(소켓 값 · 기본 = ""). */
  members: string[];
  /** 요청 **시작** 단조 시각(ms) — 만료·나이의 앵커. */
  startMs: number;
  receivedMs: number;
  value: unknown;
}

export interface ListTransportOptions {
  /** 단조 시계(ms) — 앱은 performance.now(). */
  now: () => number;
  /** D = 기존 T_LIST — 이 시간 안에 온 응답만 소비자 칸에 넣는다(넘으면 '이번 회차 데이터 없음' · 원 호출은 그대로 소유). */
  deadlineMs: number;
  /** 읽기 전용 해석(로스터 판이 바뀔 때 한 번) — 항목마다 해석된 명시 경로 문자열 또는 null(표현 불가). */
  resolve: (endpoints: (string | null)[]) => Promise<unknown>;
  /** 응답 하나의 독립 투영(다른 끝점을 기다리지 않는다). 던져도 그 응답에서 끝난다. */
  onArrival: (a: ListArrival) => void;
}

export type ListLaunch = (socket: string | undefined) => Promise<unknown>;

/** 로스터 구성원 키 — main.ts 의 `sk ?? ""`(paneKey 와 같은 규칙: 기본 소켓 = ""). */
export const memberKey = (socket: string | null | undefined): string => socket ?? "";
const memberSocket = (m: string): string | undefined => (m === "" ? undefined : m);

interface Flight {
  /** null = 기동 원 호출을 아직 끝점에 결속하지 못했다(첫 로스터 해석 전). */
  endpoint: string | null;
  member: string;
  lifetime: number;
  startMs: number;
}
interface Binding {
  endpoint: string;
  arg: string | undefined;
  resolved: boolean;
}

export class SurfaceListTransport {
  private readonly o: ListTransportOptions;
  /** 끝점 → 아직 끝나지 않은 원 호출들(정상 경로에서는 많아야 1개 · 기동 별칭 인계에서만 2개 이상). 비면 키를 지운다. */
  private readonly owners = new Map<string, Set<Flight>>();
  /** 기동(start())이 보낸 원 호출 — 끝날 때까지 추적한다. */
  private readonly startup = new Set<Flight>();
  /** 현재 끝점 → { 디스패치 인자, 수명 }. */
  private readonly endpoints = new Map<string, { arg: string | undefined; lifetime: number }>();
  private binding = new Map<string, Binding>();
  /** 구성원 → 최신 미소비 응답(한 번만 꺼낸다). */
  private readonly slots = new Map<string, ListArrival>();
  /** 이번 회차에 보낸 원 호출의 정산 promise(소비자가 상한까지 기다린다). */
  private round = new Map<string, Promise<void>>();
  private rosterSig: string | null = null;
  private rev = 0;
  private boundRev = 0;
  private pending: Promise<void> = Promise.resolve();
  private deferred: ListLaunch | null = null;
  private lifetimeSeq = 0;

  constructor(opts: ListTransportOptions) {
    this.o = opts;
  }

  /** 로스터(워크스페이스 소켓 값들)를 알린다. 판이 바뀌었을 때만 한 번 해석을 요청한다(같은 판이면 무동작). */
  setRoster(sockets: readonly (string | null | undefined)[]): void {
    const keys = [...new Set(sockets.map(memberKey))];
    const sig = JSON.stringify(keys);
    if (sig === this.rosterSig) return;
    this.rosterSig = sig;
    const rev = ++this.rev;
    let res: Promise<unknown>;
    try {
      res = Promise.resolve(this.o.resolve(keys.map((k) => memberSocket(k) ?? null)));
    } catch (e) {
      res = Promise.reject(e);
    }
    this.pending = res.then(
      (r) => this.bind(rev, keys, r),
      () => this.bind(rev, keys, null),
    );
  }

  /** 소유하지 않은 끝점마다 원 호출 하나를 **독립으로** 보낸다(직렬 await·완료 장벽 없음). 해석 대기 중이면 결속 직후로 미룬다. */
  dispatch(launch: ListLaunch): void {
    if (this.boundRev !== this.rev) {
      this.deferred = launch;
      return;
    }
    this.launchAll(launch);
  }

  private launchAll(launch: ListLaunch): void {
    this.round = new Map();
    for (const [endpoint, ep] of this.endpoints) {
      if (this.owners.has(endpoint)) continue; // 끝점당 미정산 원 호출 1개 — 상한·나이로 대체하지 않는다
      try {
        const f: Flight = { endpoint, member: "", lifetime: ep.lifetime, startMs: this.o.now() };
        const original = Promise.resolve(launch(ep.arg));
        this.own(f);
        const settled = original.then(
          (v) => this.settle(f, true, v),
          () => this.settle(f, false, undefined),
        );
        this.round.set(endpoint, settled);
      } catch {
        /* 동기 실패는 그 끝점에서 끝난다 — 소유도 없다 */
      }
    }
  }

  private own(f: Flight): void {
    if (f.endpoint === null) return;
    let set = this.owners.get(f.endpoint);
    if (!set) this.owners.set(f.endpoint, (set = new Set()));
    set.add(f);
  }

  /** 그 원 호출 **자신의** 소유만 내려놓는다(토큰 소유 해제). */
  private release(f: Flight): void {
    this.startup.delete(f);
    if (f.endpoint === null) return;
    const set = this.owners.get(f.endpoint);
    if (set && set.delete(f) && set.size === 0) this.owners.delete(f.endpoint);
  }

  private settle(f: Flight, ok: boolean, value: unknown): void {
    this.release(f);
    if (!ok || f.endpoint === null) return;
    const ep = this.endpoints.get(f.endpoint);
    if (!ep || ep.lifetime !== f.lifetime) return; // 그 사이 제거·재추가 — 옛 수명의 응답은 아무것도 되살리지 못한다
    const receivedMs = this.o.now();
    const members = [...this.binding].filter(([, b]) => b.endpoint === f.endpoint).map(([m]) => m);
    const a: ListArrival = { endpoint: f.endpoint, lifetime: f.lifetime, members, startMs: f.startMs, receivedMs, value };
    if (receivedMs - f.startMs <= this.o.deadlineMs) for (const m of members) this.slots.set(m, a);
    try {
      this.o.onArrival(a);
    } catch {
      /* 투영 실패는 그 응답에서 끝난다 */
    }
  }

  private bind(rev: number, keys: string[], res: unknown): void {
    if (rev !== this.rev) return; // 지난 로스터 판의 해석 — 적용하지 않는다(비행 도중 기본값 재계산 금지)
    const list = Array.isArray(res) && res.length === keys.length ? (res as unknown[]) : null;
    const next = new Map<string, Binding>();
    keys.forEach((k, i) => {
      const r = list?.[i];
      next.set(
        k,
        typeof r === "string" && r.length > 0
          ? { endpoint: r, arg: r, resolved: true }
          : { endpoint: `raw:${k}`, arg: memberSocket(k), resolved: false },
      );
    });
    const live = new Set([...next.values()].map((b) => b.endpoint));
    for (const e of [...this.endpoints.keys()]) {
      if (!live.has(e)) this.endpoints.delete(e); // 수명 끝 — 소유(미정산 원 호출)는 실제로 끝날 때까지 owners 에 남는다
    }
    for (const b of next.values()) {
      if (!this.endpoints.has(b.endpoint)) this.endpoints.set(b.endpoint, { arg: b.arg, lifetime: ++this.lifetimeSeq });
    }
    this.binding = next;
    for (const [m, a] of [...this.slots]) {
      const b = next.get(m);
      if (!b || b.endpoint !== a.endpoint || this.endpoints.get(b.endpoint)?.lifetime !== a.lifetime) this.slots.delete(m);
    }
    for (const f of this.startup) {
      if (f.endpoint !== null) continue;
      const b = next.get(f.member);
      if (!b) continue;
      f.endpoint = b.endpoint;
      f.lifetime = this.endpoints.get(b.endpoint)?.lifetime ?? -1;
      this.own(f); // 기동 원 호출이 첫 주기 디스패치보다 먼저 그 끝점을 소유한다
    }
    this.boundRev = rev;
    const l = this.deferred;
    this.deferred = null;
    if (l) this.launchAll(l);
  }

  /** start() 의 기동 원 호출을 넘겨받는다 — 끝날 때까지 그 소켓의 끝점을 소유한다(데이터는 start() 의 것 · 투영 없음). */
  adoptStartup(socket: string | null | undefined, original: Promise<unknown>): void {
    const f: Flight = { endpoint: null, member: memberKey(socket), lifetime: -1, startMs: this.o.now() };
    this.startup.add(f);
    const b = this.boundRev === this.rev ? this.binding.get(f.member) : undefined;
    if (b) {
      f.endpoint = b.endpoint;
      f.lifetime = this.endpoints.get(b.endpoint)?.lifetime ?? -1;
      this.own(f);
    }
    void Promise.resolve(original).then(
      () => this.release(f),
      () => this.release(f),
    );
  }

  /** 소비자 — 그 소켓의 끝점에 **이번 회차에 보낸** 원 호출이 끝나기를 기다린다(없으면 즉시). 거부하지 않는다. */
  settledFor(socket: string | null | undefined): Promise<void> {
    const m = memberKey(socket);
    if (this.boundRev !== this.rev) return this.pending.then(() => this.roundFor(m));
    return this.roundFor(m);
  }

  private roundFor(m: string): Promise<void> {
    const b = this.binding.get(m);
    return (b && this.round.get(b.endpoint)) || Promise.resolve();
  }

  /** 최신 미소비 응답을 한 번만 꺼낸다(없으면 null — 진행 중·상한 초과·오류·이미 소비). */
  take(socket: string | null | undefined): ListArrival | null {
    const m = memberKey(socket);
    const a = this.slots.get(m);
    if (!a) return null;
    this.slots.delete(m);
    return a;
  }

  /** 추적 중인 미정산 원 호출 수(정규 + 아직 결속 못 한 기동 원 호출). */
  outstanding(): number {
    const all = new Set<Flight>();
    for (const s of this.owners.values()) for (const f of s) all.add(f);
    for (const f of this.startup) all.add(f);
    return all.size;
  }

  outstandingFor(endpoint: string): number {
    return this.owners.get(endpoint)?.size ?? 0;
  }

  endpointOf(socket: string | null | undefined): string | undefined {
    return this.binding.get(memberKey(socket))?.endpoint;
  }

  lifetimeOf(endpoint: string): number | undefined {
    return this.endpoints.get(endpoint)?.lifetime;
  }

  /** Members that could not be resolved (alias identity not proven) — an introspection accessor; no production caller reports them. */
  unresolved(): string[] {
    return [...this.binding].filter(([, b]) => !b.resolved).map(([m]) => m);
  }
}
