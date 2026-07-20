// Thin wasm-bindgen wrapper around crdt-core so the browser client runs the
// exact same RGA implementation as the server - the whole point of the shared
// crate is that client and server algorithm logic can never silently diverge.
// The API surface is deliberately small and batched (arrays of ops in/out, not
// one call per keystroke) to keep the JS<->WASM boundary cheap.

use crdt_core::{Op, RgaDoc};
use wasm_bindgen::prelude::*;

/// Route Rust panics to the browser console with real stack info instead of an
/// opaque "unreachable executed" trap - call this once from the client on load.
#[wasm_bindgen(js_name = initPanicHook)]
pub fn init_panic_hook() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub struct CrdtClient {
    doc: RgaDoc,
}

#[wasm_bindgen]
impl CrdtClient {
    #[wasm_bindgen(constructor)]
    pub fn new(site_id: u64) -> CrdtClient {
        CrdtClient { doc: RgaDoc::new(site_id) }
    }

    pub fn render_text(&self) -> String {
        self.doc.render_text()
    }

    /// Insert `value` (a single character) at visible-text position `pos`.
    /// Returns the produced op, JSON-serialized, for the caller to broadcast.
    #[wasm_bindgen(js_name = localInsert)]
    pub fn local_insert(&mut self, pos: usize, value: char) -> Result<String, JsError> {
        let op = self.doc.local_insert(pos, value);
        serde_json::to_string(&op).map_err(to_js_err)
    }

    #[wasm_bindgen(js_name = localDelete)]
    pub fn local_delete(&mut self, pos: usize) -> Result<Option<String>, JsError> {
        match self.doc.local_delete(pos) {
            Some(op) => serde_json::to_string(&op).map(Some).map_err(to_js_err),
            None => Ok(None),
        }
    }

    #[wasm_bindgen(js_name = undo)]
    pub fn undo(&mut self) -> Result<Option<String>, JsError> {
        match self.doc.undo() {
            Some(op) => serde_json::to_string(&op).map(Some).map_err(to_js_err),
            None => Ok(None),
        }
    }

    #[wasm_bindgen(js_name = redo)]
    pub fn redo(&mut self) -> Result<Option<String>, JsError> {
        match self.doc.redo() {
            Some(op) => serde_json::to_string(&op).map(Some).map_err(to_js_err),
            None => Ok(None),
        }
    }

    /// Apply a batch of remote ops (as a JSON array), e.g. everything received
    /// since the last call - never invoked per-op from JS.
    #[wasm_bindgen(js_name = applyRemoteOps)]
    pub fn apply_remote_ops(&mut self, ops_json: &str) -> Result<(), JsError> {
        let ops: Vec<Op> = serde_json::from_str(ops_json).map_err(to_js_err)?;
        self.doc.apply_remote_ops(ops);
        Ok(())
    }

    #[wasm_bindgen(js_name = stateVector)]
    pub fn state_vector(&self) -> Result<String, JsError> {
        serde_json::to_string(&self.doc.state_vector()).map_err(to_js_err)
    }
}

fn to_js_err<E: std::fmt::Display>(e: E) -> JsError {
    JsError::new(&e.to_string())
}
