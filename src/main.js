// Tauri 浏览器 — 前端逻辑
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ─── DOM 元素 ───

const urlInput = document.getElementById("url-input");
const btnGo = document.getElementById("btn-go");
const btnDownloads = document.getElementById("btn-downloads");
const downloadPanel = document.getElementById("download-panel");
const downloadList = document.getElementById("download-list");
const btnClosePanel = document.getElementById("btn-close-panel");
const mediaToast = document.getElementById("media-toast");
const toastMsg = document.getElementById("toast-msg");
const toastDownload = document.getElementById("toast-download");
const toastDismiss = document.getElementById("toast-dismiss");
const homePage = document.getElementById("home-page");

// ─── 下载状态 ───

const downloads = [];
let currentMediaUrl = null;
let currentMediaFilename = null;

// ─── 导航 ───

async function doNavigate(url) {
  if (!url.trim()) return;

  // 隐藏首页
  homePage.classList.add("hidden");
  urlInput.blur();

  try {
    await invoke("navigate", { url: url.trim() });
  } catch (err) {
    console.error("导航失败:", err);
    alert("导航失败: " + err);
  }
}

btnGo.addEventListener("click", () => doNavigate(urlInput.value));
urlInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") doNavigate(urlInput.value);
});

// ─── 快捷链接 ───

document.querySelectorAll(".quick-links a").forEach((link) => {
  link.addEventListener("click", (e) => {
    e.preventDefault();
    const url = link.dataset.url;
    urlInput.value = url;
    doNavigate(url);
  });
});

// ─── 下载面板 ───

btnDownloads.addEventListener("click", () => {
  downloadPanel.classList.toggle("hidden");
  renderDownloadList();
});

btnClosePanel.addEventListener("click", () => {
  downloadPanel.classList.add("hidden");
});

function renderDownloadList() {
  if (downloads.length === 0) {
    downloadList.innerHTML = '<p class="empty-msg">暂无下载</p>';
    return;
  }
  downloadList.innerHTML = downloads
    .map(
      (d, i) => `
    <div class="dl-item ${d.done ? 'done' : ''}">
      <div class="dl-info">
        <div class="dl-name">📦 ${d.filename}</div>
        <div class="dl-progress">
          ${d.done ? '✅ 完成: ' + d.path : '⏳ ' + d.progress}
        </div>
      </div>
      ${d.done ? '<span class="dl-badge">完成</span>' : ''}
    </div>`
    )
    .join("");
}

// ─── 媒体检测通知条 ───

toastDownload.addEventListener("click", async () => {
  if (!currentMediaUrl) return;
  mediaToast.classList.add("hidden");

  const filename = currentMediaFilename || "video.mp4";

  // 添加下载任务
  downloads.push({
    url: currentMediaUrl,
    filename: filename,
    done: false,
    progress: "0%",
    path: "",
  });
  renderDownloadList();

  try {
    await invoke("start_download", {
      url: currentMediaUrl,
      filename: filename,
    });
  } catch (err) {
    console.error("下载启动失败:", err);
  }
});

toastDismiss.addEventListener("click", () => {
  mediaToast.classList.add("hidden");
  currentMediaUrl = null;
});

// ─── 监听 Rust 事件 ───

// 媒体检测到
listen("media-detected", (event) => {
  const { url, filename, mediaType } = event.payload;

  currentMediaUrl = url;
  currentMediaFilename = filename;

  toastMsg.textContent = `🎬 检测到${mediaType === "audio" ? "音频" : "视频"}: ${filename}`;
  mediaToast.classList.remove("hidden");

  // 3 秒自动隐藏
  clearTimeout(mediaToast._timeout);
  mediaToast._timeout = setTimeout(() => {
    mediaToast.classList.add("hidden");
  }, 8000);
});

// 下载进度
listen("download-progress", (event) => {
  const { url, filename, downloaded, total, done, path } = event.payload;

  // 更新或创建下载记录
  let dl = downloads.find((d) => d.url === url);
  if (!dl) {
    dl = { url, filename, done: false, progress: "0%", path: "" };
    downloads.unshift(dl);
  }

  if (done) {
    dl.done = true;
    dl.path = path;
    dl.progress = "100%";
  } else {
    const dlMB = (downloaded / 1048576).toFixed(1);
    const totalMB = total > 0 ? (total / 1048576).toFixed(1) : "?";
    dl.progress = `${dlMB} / ${totalMB} MB`;
  }

  renderDownloadList();

  // 下载完成，短暂显示面板
  if (done && !path.startsWith("错误")) {
    downloadPanel.classList.remove("hidden");
    setTimeout(() => downloadPanel.classList.add("hidden"), 4000);
  }
});

// ─── URL 自动补全 ───

urlInput.addEventListener("focus", () => {
  if (!urlInput.value) {
    urlInput.placeholder = "输入网址，例如: bilibili.com";
  }
});

// ─── 初始化 ───

console.log("🌐 Tauri 浏览器已就绪");
