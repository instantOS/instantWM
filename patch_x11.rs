--- rust/src/backend/x11/mod.rs
+++ rust/src/backend/x11/mod.rs
@@ -107,4 +107,33 @@
             );
         }
     }
+
+    fn list_displays(&self) -> Vec<String> {
+        if let Ok(output) = std::process::Command::new("xrandr").output() {
+            let stdout = String::from_utf8_lossy(&output.stdout);
+            stdout.lines()
+                .filter(|line| line.contains(" connected"))
+                .map(|line| line.split_whitespace().next().unwrap_or("").to_string())
+                .collect()
+        } else {
+            vec![]
+        }
+    }
+
+    fn list_display_modes(&self, _display: &str) -> Vec<String> {
+        if let Ok(output) = std::process::Command::new("xrandr").output() {
+            let _stdout = String::from_utf8_lossy(&output.stdout);
+            // In a real implementation we would parse xrandr output more carefully
+            // For now this is a placeholder stub
+            vec!["1920x1080@60.0".to_string()]
+        } else {
+            vec![]
+        }
+    }
+
+    fn set_display_mode(&self, display: &str, width: i32, height: i32) {
+        let mode = format!("{}x{}", width, height);
+        let _ = std::process::Command::new("xrandr")
+            .args(["--output", display, "--mode", &mode])
+            .status();
+    }
 }
