import { Editor } from "./Editor";
import { useCrdtClient } from "./useCrdtClient";
import "./App.css";

// Milestone 3: the editor is now backed by crdt-core running in WASM, synced
// over a WebSocket to the axum server (Milestone 2's naive relay, replaced with
// real CRDT ops). Open this page in two tabs with the same ?doc= to see
// concurrent edits converge live.
function docIdFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("doc") ?? "demo";
}

export default function App() {
  const docId = docIdFromUrl();
  const { text, status, applyLocalOps, undo, redo } = useCrdtClient(docId);

  return (
    <div className="app-shell">
      <header>
        <h1>CRDT Editor</h1>
        <p className="subtitle">
          doc: <code>{docId}</code> &middot; <span className={`status status-${status}`}>{status}</span>
        </p>
      </header>
      <div className="toolbar">
        <button type="button" onClick={undo}>
          Undo
        </button>
        <button type="button" onClick={redo}>
          Redo
        </button>
      </div>
      <Editor
        initialText={text}
        remoteText={text}
        onLocalOps={applyLocalOps}
        onUndo={undo}
        onRedo={redo}
      />
    </div>
  );
}
