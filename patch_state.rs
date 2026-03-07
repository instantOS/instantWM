--- rust/src/backend/wayland/compositor/state.rs
+++ rust/src/backend/wayland/compositor/state.rs
@@ -364,6 +364,28 @@
         output
     }

+    pub fn list_displays(&self) -> Vec<String> {
+        self.space.outputs().map(|o| o.name()).collect()
+    }
+
+    pub fn list_display_modes(&self, display: &str) -> Vec<String> {
+        let mut result = Vec::new();
+        if let Some(output) = self.space.outputs().find(|o| o.name() == display) {
+            for mode in output.modes() {
+                result.push(format!("{}x{}@{}", mode.size.w, mode.size.h, mode.refresh as f64 / 1000.0));
+            }
+        }
+        result
+    }
+
+    pub fn set_display_mode(&mut self, display: &str, width: i32, height: i32) {
+        if let Some(output) = self.space.outputs().find(|o| o.name() == display).cloned() {
+            if let Some(mode) = output.modes().into_iter().find(|m| m.size.w == width && m.size.h == height) {
+                output.change_current_state(Some(mode), None, None, None);
+            }
+        }
+    }
+
     pub fn sync_space_from_globals(&mut self) {
         let Some(g) = self.globals() else {
             return;
