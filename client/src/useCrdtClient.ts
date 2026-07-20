import { useCallback, useEffect, useRef, useState } from "react";
import init, { CrdtClient, initPanicHook } from "./wasm/client_wasm.js";
import type { LocalOp } from "./editorOps";

type ServerMsg =
  | { type: "welcome"; site_id: number; ops: unknown[] }
  | { type: "op"; op: unknown };

export type ConnectionStatus = "connecting" | "connected" | "disconnected";

const WS_URL = (docId: string) => `ws://localhost:8787/ws/${docId}`;

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
      for (const op of ops) {
        if (op.kind === "insert") {
          // The editor gives us a whole inserted string (e.g. a paste); crdt-core
          // inserts one character at a time, each becoming its own RGA node.
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
