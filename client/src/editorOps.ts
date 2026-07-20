// The editor <-> CRDT boundary. CodeMirror gives us position-based change events
// (not raw DOM mutations), so each keystroke maps directly to a small set of
// LocalOps here, which useCrdtClient routes into the crdt-core WASM module one
// character at a time.

export type LocalOp =
  | { kind: "insert"; pos: number; text: string }
  | { kind: "delete"; pos: number; len: number };
