import { useEffect, useRef } from "react";
import { Annotation, EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine } from "@codemirror/view";
import { defaultKeymap } from "@codemirror/commands";
import type { LocalOp } from "./editorOps";

/// Marks a transaction as "this text came from the CRDT, not the user", so the
/// update listener doesn't feed it straight back in as a fresh local edit. An
/// annotation rather than a mutable ref: it travels with the transaction itself,
/// so it stays correct even if a dispatch ever becomes async or batched.
const remoteChange = Annotation.define<boolean>();

const isLowSurrogate = (ch: string | undefined) =>
  ch !== undefined && ch >= "\uDC00" && ch <= "\uDFFF";

/**
 * Smallest single replacement that turns `current` into `next`, as a
 * common-prefix / common-suffix trim.
 *
 * Replacing the whole document instead would be simpler, but CodeMirror maps the
 * user's selection through whatever change we dispatch - so a full-document
 * replace drags every collaborator's cursor to the end of the file on every
 * remote keystroke. A minimal change leaves cursors outside the edited span
 * exactly where they were.
 */
export function minimalChange(current: string, next: string) {
  let start = 0;
  const max = Math.min(current.length, next.length);
  while (start < max && current[start] === next[start]) start++;

  let endCur = current.length;
  let endNext = next.length;
  while (endCur > start && endNext > start && current[endCur - 1] === next[endNext - 1]) {
    endCur--;
    endNext--;
  }

  // Never let a boundary land between a surrogate pair, or the dispatched change
  // would split an emoji into two lone surrogates.
  if (start > 0 && (isLowSurrogate(current[start]) || isLowSurrogate(next[start]))) {
    start--;
  }
  if (isLowSurrogate(current[endCur]) || isLowSurrogate(next[endNext])) {
    endCur = Math.min(endCur + 1, current.length);
    endNext = Math.min(endNext + 1, next.length);
  }

  return { from: start, to: endCur, insert: next.slice(start, endNext) };
}

interface EditorProps {
  initialText: string;
  onLocalOps: (ops: LocalOp[]) => void;
  /** Latest text rendered by the CRDT; pushed into the view as a minimal change. */
  remoteText?: string;
  /** CRDT undo/redo - deliberately not CodeMirror's local history (see below). */
  onUndo: () => void;
  onRedo: () => void;
}

export function Editor({ initialText, onLocalOps, remoteText, onUndo, onRedo }: EditorProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);

  // The view is constructed once; route callbacks through refs so it always calls
  // the current props without tearing down and rebuilding the editor.
  const onLocalOpsRef = useRef(onLocalOps);
  const onUndoRef = useRef(onUndo);
  const onRedoRef = useRef(onRedo);
  onLocalOpsRef.current = onLocalOps;
  onUndoRef.current = onUndo;
  onRedoRef.current = onRedo;

  useEffect(() => {
    if (!hostRef.current) return;

    const state = EditorState.create({
      doc: initialText,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        // NB: no history()/historyKeymap here. CodeMirror's undo stack is
        // position-based and local-only, which is exactly what a CRDT undo must
        // not be - it would revert text without emitting the compensating CRDT
        // ops, so other clients would never see the undo and the two stacks would
        // drift apart. Mod-z is bound to the CRDT's own undo instead, and is
        // listed before defaultKeymap so it takes precedence.
        keymap.of([
          { key: "Mod-z", preventDefault: true, run: () => (onUndoRef.current(), true) },
          { key: "Mod-Shift-z", preventDefault: true, run: () => (onRedoRef.current(), true) },
          { key: "Mod-y", preventDefault: true, run: () => (onRedoRef.current(), true) },
        ]),
        keymap.of(defaultKeymap),
        EditorView.lineWrapping,
        EditorView.updateListener.of((update) => {
          if (!update.docChanged) return;
          if (update.transactions.some((tr) => tr.annotation(remoteChange))) return;
          const ops: LocalOp[] = [];
          update.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
            if (toA > fromA) {
              ops.push({ kind: "delete", pos: fromA, len: toA - fromA });
            }
            if (inserted.length > 0) {
              ops.push({ kind: "insert", pos: fromA, text: inserted.toString() });
            }
          });
          if (ops.length > 0) onLocalOpsRef.current(ops);
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
    view.dispatch({
      changes: minimalChange(current, remoteText),
      annotations: remoteChange.of(true),
    });
  }, [remoteText]);

  return <div className="editor-host" ref={hostRef} />;
}
