import { randomBytes } from "node:crypto";
import { CAST_PROTOCOL_VERSION } from "../cast";

let generationSeq = 0;

export interface IssuedCastEmbed {
  appUrl: string;
  wsUrl: string;
  ticket: string;
  generation: number;
  parentOrigin: string;
}

// 실제 GUI와 같은 app GET(issue) → WS(consume) 순서를 테스트에서 재사용한다. bare WS 우회는 없다.
export async function issueCastEmbed(port: number, token: string, context = "default"): Promise<IssuedCastEmbed> {
  const generation = ++generationSeq;
  const ticket = randomBytes(32).toString("hex");
  const parentOrigin = process.platform === "win32" ? "http://tauri.localhost" : "tauri://localhost";
  const query = new URLSearchParams({
    context,
    protocolVersion: String(CAST_PROTOCOL_VERSION),
    embedGeneration: String(generation),
    parentOrigin,
    embedTicket: ticket,
  });
  const appUrl = `http://127.0.0.1:${port}/${token}/cast/?${query.toString()}`;
  const response = await fetch(appUrl);
  if (!response.ok) throw new Error(`cast ticket issue failed: ${response.status} ${await response.text()}`);
  return {
    appUrl,
    wsUrl: `ws://127.0.0.1:${port}/${token}/cast/ws?${query.toString()}`,
    ticket,
    generation,
    parentOrigin,
  };
}
