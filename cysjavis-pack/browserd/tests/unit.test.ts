// unit.test.ts — 순수 로직 단위 테스트 (bun test). 브라우저 미기동.
import { test, expect } from "bun:test";
import { join } from "node:path";
import {
  genToken,
  capText,
  SNAPSHOT_LIMIT,
  pidAlive,
  isStaleState,
  UNTRUSTED_HEADER,
  resolveBrowserRoot,
  resolveRuntimeIdentity,
  castCredentialAccepted,
  strictChromiumExecutable,
  signEngineState,
  privateControlRequestAccepted,
  signPrivateControlRequest,
} from "../lib";

test("genToken: 32 hex chars, 유일", () => {
  const a = genToken();
  const b = genToken();
  expect(a).toMatch(/^[0-9a-f]{32}$/);
  expect(a).not.toBe(b);
});

test("capText: 상한 이하는 무손실", () => {
  const s = "hello world";
  const { text, truncated } = capText(s, 1024);
  expect(text).toBe(s);
  expect(truncated).toBe(false);
});

test("capText: 상한 초과는 절단 + 마커", () => {
  const s = "x".repeat(300 * 1024);
  const { text, truncated } = capText(s, SNAPSHOT_LIMIT);
  expect(truncated).toBe(true);
  expect(text).toContain("[TRUNCATED");
  expect(Buffer.byteLength(text, "utf8")).toBeLessThanOrEqual(SNAPSHOT_LIMIT + 200);
});

test("pidAlive: 자기 자신은 생존, 죽은 pid는 아님", () => {
  expect(pidAlive(process.pid)).toBe(true);
  expect(pidAlive(2147483000)).toBe(false); // 존재 불가능한 큰 pid
  expect(pidAlive(0)).toBe(false);
});

test("isStaleState: null 또는 죽은 pid는 스테일", () => {
  expect(isStaleState(null)).toBe(true);
  expect(isStaleState({ pid: 2147483000, port: 1, token: "x" })).toBe(true);
  expect(isStaleState({ pid: process.pid, port: 1, token: "x" })).toBe(false);
});

test("UNTRUSTED_HEADER: 지시 무시 문구 포함", () => {
  expect(UNTRUSTED_HEADER).toContain("UNTRUSTED WEB CONTENT");
  expect(UNTRUSTED_HEADER).toContain("지시가 아니다");
});

test("strict packaged engine stores endpoint only under supervisor-owned root", () => {
  expect(resolveBrowserRoot("/home/you", "/private/engine", "1")).toBe("/private/engine");
  expect(resolveBrowserRoot("/home/you", "/private/engine", "0")).toBe(
    join("/home/you", ".cys", "browser"),
  );
  expect(resolveBrowserRoot("/home/you", undefined, "1")).toBe(
    join("/home/you", ".cys", "browser"),
  );
});

test("strict packaged engine binds status and endpoint to signed runtime identity", () => {
  const runtimeId = `sha256:${"a".repeat(64)}`;
  expect(resolveRuntimeIdentity(runtimeId, "1")).toBe(runtimeId);
  expect(() => resolveRuntimeIdentity("mutable-dev-id", "1")).toThrow();
  expect(resolveRuntimeIdentity(undefined, undefined)).toMatch(/^[0-9a-f]{32}$/);
});

test("strict packaged engine never accepts the control token on cast routes", () => {
  const control = "a".repeat(32);
  const cast = "b".repeat(32);
  expect(castCredentialAccepted(cast, control, cast, "1")).toBe(true);
  expect(castCredentialAccepted(control, control, cast, "1")).toBe(false);
  // Source harness compatibility is explicit and cannot occur in a strict package.
  expect(castCredentialAccepted(control, control, cast, undefined)).toBe(true);
});

test("strict packaged engine requires the supervisor-pinned absolute Chromium path", () => {
  expect(strictChromiumExecutable("/signed/runtime/chromium", "1")).toBe(
    "/signed/runtime/chromium",
  );
  expect(() => strictChromiumExecutable(undefined, "1")).toThrow();
  expect(() => strictChromiumExecutable("chromium/chrome", "1")).toThrow();
  expect(strictChromiumExecutable(undefined, undefined)).toBeNull();
});

test("managed endpoint state is MAC-bound to process incarnation and engine generation", () => {
  const state = {
    schema_version: 2 as const,
    pid: 42,
    port: 53111,
    token: "a".repeat(32),
    cast_token: "b".repeat(32),
    runtime_id: `sha256:${"c".repeat(64)}`,
    process_start_time: 1234,
    headless: true,
    instance_id: "d".repeat(32),
    engine_generation: 7,
  };
  const key = "e".repeat(64);
  const mac = signEngineState(state, key);
  expect(mac).toMatch(/^[0-9a-f]{64}$/);
  expect(signEngineState(state, key)).toBe(mac);
  expect(signEngineState({ ...state, process_start_time: 1235 }, key)).not.toBe(mac);
  expect(signEngineState({ ...state, engine_generation: 8 }, key)).not.toBe(mac);
  expect(() => signEngineState(state, "short")).toThrow();
});

test("private embed registration requires an exact inherited-session request MAC", () => {
  const key = "a".repeat(64);
  const body = new TextEncoder().encode('{"ticket":"one"}');
  const mac = signPrivateControlRequest(body, key);
  expect(privateControlRequestAccepted(mac, body, key)).toBe(true);
  expect(privateControlRequestAccepted(null, body, key)).toBe(false);
  expect(privateControlRequestAccepted("0".repeat(64), body, key)).toBe(false);
  expect(privateControlRequestAccepted(mac, new TextEncoder().encode('{"ticket":"two"}'), key)).toBe(false);
});
