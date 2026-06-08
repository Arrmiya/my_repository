// Tauri API for invoking Rust commands from the frontend
import { invoke } from "@tauri-apps/api/core";

// Prevent the default right-click menu in production
document.addEventListener("contextmenu", (e) => {
  if (!import.meta.env.DEV) e.preventDefault();
});

// Greet button — calls the Rust backend
const greetBtn = document.getElementById("greet-btn");
const greetMsg = document.getElementById("greet-msg");

if (greetBtn) {
  greetBtn.addEventListener("click", async () => {
    try {
      const response = await invoke("greet", { name: "Android 用户" });
      greetMsg.textContent = response;
      greetMsg.classList.add("visible");
    } catch (err) {
      greetMsg.textContent = `错误: ${err}`;
      greetMsg.classList.add("visible");
    }
  });
}

// Log when the app is ready
console.log("🚀 Tauri app frontend loaded");
