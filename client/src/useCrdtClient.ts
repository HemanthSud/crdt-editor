import { useCallback, useEffect, useRef, useState } from "react";
import init, { CrdtClient, initPanicHook } from "./wasm/client_wasm.js";
import type { LocalOp } from "./editorOps";

type ServerMsg =
  | { type: "welcome"; site_id: number; ops: unknown[] }
  | { type: "op"; op: unknown };

export type ConnectionStatus = "connecting" | "connected" | "disconnected";

const WS_BASE = import.meta.env.VITE_WS_URL ?? "ws://localhost:8787";
const WS_URL = (docId: string) => `${WS_BASE}/ws/${encodeURIComponent(docId)}`;

/**
 * CodeMirror measures positions in UTF-16 code units; `crdt-core` indexes by
 * Unicode scalar value (Rust `char`). They agree for the BMP and diverge the
 * moment a document contains an emoji or any other astral character - after
 * which every subsequent op targets the wrong node. Convert at the boundary.
 */
const toCodePointIndex = (text: string, utf16Pos: number) =>
  [...text.slice(0, utf16Pos)].length;

/** Number of code points in a UTF-16 span - a delete length, in CRDT terms. */
const codePointLength = (text: string, from: number, to: number) =>
  [...text.slice(from, to)].length;

/**
 * Owns one CrdtClient (crdt-core compiled to WASM) plus the WebSocket that
 * feeds it remote ops. `text` reflects the CRDT's render_text() after every
 * local or remote op - the editor is a dumb view over it.
 */
export function useCrdtClient(docId: string) {
  const [text, setText] = useState("");
  const [status, setStatus] = useState<ConnectionStatus>("connecting");
  const clientRef = useRef<CrdtClient | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const readyRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    let ws: WebSocket | null = null;

    (async () => {
      await init();
      initPanicHook();
      if (cancelled) return;

      ws = new WebSocket(WS_URL(docId));
      wsRef.current = ws;

      ws.onopen = () => setStatus("connected");
      ws.onclose = () => setStatus("disconnected");
      ws.onerror = () => setStatus("disconnected");

      ws.onmessage = (event) => {
        const msg: ServerMsg = JSON.parse(event.data);
        if (msg.type === "welcome") {
          const client = new CrdtClient(BigInt(msg.site_id));
          clientRef.current = client;
          if (msg.ops.length > 0) {
            client.applyRemoteOps(JSON.stringify(msg.ops));
          }
          setText(client.render_text());
          readyRef.current = true;
        } else if (msg.type === "op") {
          const client = clientRef.current;
          if (!client) return; // op arrived before welcome - shouldn't happen, server sends welcome first
          client.applyRemoteOps(JSON.stringify([msg.op]));
          setText(client.render_text());
        }
      };
    })();

    return () => {
      cancelled = true;
      ws?.close();
    };
  }, [docId]);

  const sendOp = useCallback((opJson: string | undefined | null) => {
    if (!opJson) return;
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "op", op: JSON.parse(opJson) }));
    }
  }, []);

  const applyLocalOps = useCallback(
    (ops: LocalOp[]) => {
      const client = clientRef.current;
      if (!client || !readyRef.current) return;

      // CodeMirror reports every change in one transaction against the *same*
      // pre-edit document, so all positions must be converted against that same
      // base text before we start mutating the CRDT.
      const base = client.render_text();
      const converted: LocalOp[] = ops.map((op) =>
        op.kind === "insert"
          ? { kind: "insert", pos: toCodePointIndex(base, op.pos), text: op.text }
          : {
              kind: "delete",
              pos: toCodePointIndex(base, op.pos),
              len: codePointLength(base, op.pos, op.pos + op.len),
            },
      );

      // Apply back-to-front: each op shifts the positions after it, so starting
      // from the highest position keeps every remaining (pre-edit) position valid.
      // Array.prototype.sort is stable, so a delete+insert pair at the same
      // position (a replacement) keeps its delete-then-insert order.
      converted.sort((a, b) => b.pos - a.pos);

      for (const op of converted) {
        if (op.kind === "insert") {
          // The editor gives us a whole inserted string (e.g. a paste); crdt-core
          // inserts one code point at a time, each becoming its own RGA node.
          let pos = op.pos;
          for (const ch of op.text) {
            const opJson = client.localInsert(pos, ch);
            sendOp(opJson);
            pos += 1;
          }
        } else {
          for (let i = 0; i < op.len; i++) {
            const opJson = client.localDelete(op.pos);
            sendOp(opJson ?? null);
          }
        }
      }
      setText(client.render_text());
    },
    [sendOp],
  );

  const undo = useCallback(() => {
    const client = clientRef.current;
    if (!client) return;
    const opJson = client.undo();
    if (opJson) {
      sendOp(opJson);
      setText(client.render_text());
    }
  }, [sendOp]);

  const redo = useCallback(() => {
    const client = clientRef.current;
    if (!client) return;
    const opJson = client.redo();
    if (opJson) {
      sendOp(opJson);
      setText(client.render_text());
    }
  }, [sendOp]);

  return { text, status, applyLocalOps, undo, redo };
}
