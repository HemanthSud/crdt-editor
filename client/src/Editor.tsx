import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import type { LocalOp } from "./editorOps";

interface EditorProps {
  initialText: string;
  onLocalOps: (ops: LocalOp[]) => void;
  /** Bumped by the parent whenever it needs to push a non-local text change
   * (e.g. a remote CRDT op applied) into the view without re-triggering onLocalOps. */
  remoteText?: string;
}

export function Editor({ initialText, onLocalOps, remoteText }: EditorProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const applyingRemote = useRef(false);

  useEffect(() => {
    if (!hostRef.current) return;

    const state = EditorState.create({
      doc: initialText,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        EditorView.lineWrapping,
        EditorView.updateListener.of((update) => {
          if (!update.docChanged || applyingRemote.current) return;
          const ops: LocalOp[] = [];
          update.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
            if (toA > fromA) {
              ops.push({ kind: "delete", pos: fromA, len: toA - fromA });
            }
            if (inserted.length > 0) {
              ops.push({ kind: "insert", pos: fromA, text: inserted.toString() });
            }
          });
          if (ops.length > 0) onLocalOps(ops);
        }),
      ],
    });

    const view = new EditorView({ state, parent: hostRef.current });
    viewRef.current = view;
    return () => view.destroy();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || remoteText === undefined) return;
    const current = view.state.doc.toString();
    if (current === remoteText) return;
    applyingRemote.current = true;
    view.dispatch({
      changes: { from: 0, to: current.length, insert: remoteText },
    });
    applyingRemote.current = false;
  }, [remoteText]);

  return <div className="editor-host" ref={hostRef} />;
}
