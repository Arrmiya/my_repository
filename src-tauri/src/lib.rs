// Tauri v2 浏览器 + 视频嗅探下载
// 桌面入口 main.rs，Android 通过 mobile_entry_point 启动

use futures_util::StreamExt;
use reqwest::Client;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{command, AppHandle, Emitter, Manager};
use tokio::sync::Mutex;
use url::Url;

// ─── 视频检测注入脚本（注入到每个外部页面） ───

const INJECT_SCRIPT: &str = r#"
(function(){
if(window.__browserInjected)return;window.__browserInjected=1;

let detected=new Set();
function notify(url,type,title){
  let k=url+'|'+type;
  if(detected.has(k))return;
  detected.add(k);
  try{window.__TAURI__&&window.__TAURI__.invoke('detect_media',{url:url,mediaType:type,pageTitle:document.title||''});}catch(e){}
}

// 扫描 <video> / <audio> 标签
function scan(){
  document.querySelectorAll('video,audio').forEach(function(el){
    if(el.src&&el.src.startsWith('http'))notify(el.src,el.tagName.toLowerCase(),document.title);
    el.querySelectorAll('source').forEach(function(s){
      if(s.src&&s.src.startsWith('http'))notify(s.src,el.tagName.toLowerCase(),document.title);
    });
  });
}
scan();
new MutationObserver(scan).observe(document.documentElement,{childList:true,subtree:true});

// 拦截 fetch 捕获 m3u8 / mp4 请求
let _fetch=window.fetch;
window.fetch=function(url,opts){
  let u=typeof url==='string'?url:(url instanceof Request?url.url:'');
  if(/\.(m3u8|mp4|ts|m4v|mkv|webm|flv)(\?|$)/i.test(u))notify(u,'stream',document.title);
  return _fetch.apply(this,arguments);
};

// 拦截 XMLHttpRequest
let _open=XMLHttpRequest.prototype.open;
XMLHttpRequest.prototype.open=function(method,url){
  if(/\.(m3u8|mp4|ts|m4v|mkv|webm|flv)(\?|$)/i.test(url))notify(url,'xhr',document.title);
  return _open.apply(this,arguments);
};

// 浮动 UI（右下角）
let ui=document.createElement('div');
ui.id='__browser_float';
ui.innerHTML='<button id="__b_home" title="返回浏览器">🏠</button><button id="__b_dl" style="display:none" title="下载"></button>';
ui.style.cssText='position:fixed;bottom:20px;right:20px;z-index:2147483647;display:flex;flex-direction:column;gap:8px;font-size:24px;';
document.body.appendChild(ui);

document.getElementById('__b_home').onclick=function(){
  try{window.__TAURI__&&window.__TAURI__.invoke('go_home');}catch(e){}
};

// 监听下载事件，更新浮动按钮
try{
  window.__TAURI__&&window.__TAURI__.event.listen('media-detected',function(ev){
    let p=ev.payload;
    let btn=document.getElementById('__b_dl');
    btn.style.display='block';
    btn.textContent='⬇ '+p.filename+' ('+p.mediaType+')';
    btn.title=p.url;
    btn.onclick=function(){
      try{window.__TAURI__&&window.__TAURI__.invoke('start_download',{url:p.url,filename:p.filename});}catch(e){}
      btn.textContent='⏳ 下载中...';
    };
  });
  window.__TAURI__&&window.__TAURI__.event.listen('download-progress',function(ev){
    let p=ev.payload;
    let btn=document.getElementById('__b_dl');
    if(p.done){
      btn.textContent='✅ 完成';
      btn.style.display='block';
    }else{
      let mb=(p.downloaded/1048576).toFixed(1);
      let totalMb=p.total>0?(p.total/1048576).toFixed(1):'?';
      btn.textContent='⏳ '+mb+'/'+totalMb+' MB';
    }
  });
}catch(e){}

console.log('[Browser] Video detector injected');
})();
"#;

// ─── 数据结构 ───

#[derive(Clone, Serialize)]
struct MediaInfo {
    url: String,
    media_type: String,
    page_title: String,
    filename: String,
    size: String,
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    url: String,
    filename: String,
    downloaded: u64,
    total: u64,
    done: bool,
    path: String,
}

struct AppState {
    cancel_flag: Arc<AtomicBool>,
}

// ─── 命令：导航到 URL ───

#[command]
async fn navigate(app: AppHandle, url: String) -> Result<(), String> {
    let webview = app
        .get_webview_window("main")
        .ok_or("未找到主窗口")?;

    let target_url = if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else if url.contains('.') && !url.contains(' ') {
        format!("https://{}", url)
    } else {
        format!("https://www.google.com/search?q={}", url.replace(' ', "+"))
    };

    let parsed = Url::parse(&target_url).map_err(|e| format!("无效的 URL: {}", e))?;

    webview
        .navigate(parsed)
        .map_err(|e| format!("导航失败: {}", e))?;

    // 短暂延迟后注入视频检测脚本
    let wv = webview.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let _ = wv.eval(INJECT_SCRIPT);
    });

    Ok(())
}

// ─── 命令：回到主页 ───

#[command]
async fn go_home(app: AppHandle) -> Result<(), String> {
    let webview = app
        .get_webview_window("main")
        .ok_or("未找到主窗口")?;

    // 通过 JS 导航回应用首页
    let _ = webview.eval("location.href = '/'");

    Ok(())
}

// ─── 命令：前端通知检测到媒体 ───

#[command]
async fn detect_media(
    app: AppHandle,
    url: String,
    media_type: String,
    page_title: String,
) -> Result<(), String> {
    let filename = extract_filename(&url, &media_type);

    let info = MediaInfo {
        url: url.clone(),
        media_type: media_type.clone(),
        page_title,
        filename: filename.clone(),
        size: "未知".into(),
    };

    app.emit("media-detected", &info)
        .map_err(|e| format!("事件发送失败: {}", e))?;

    println!("[Browser] 检测到媒体: {} ({})", filename, url);
    Ok(())
}

// ─── 命令：开始下载 ───

#[command]
async fn start_download(
    app: AppHandle,
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
    url: String,
    filename: String,
) -> Result<String, String> {
    let state = state.lock().await;
    state.cancel_flag.store(false, Ordering::SeqCst);
    let cancel = state.cancel_flag.clone();
    drop(state);

    // 获取下载目录
    let download_dir = get_download_dir(&app)?;
    let fname = sanitize_filename(&filename);
    let file_path = download_dir.join(&fname);

    let app_handle = app.clone();
    let url_clone = url.clone();

    tokio::spawn(async move {
        match download_file(&url_clone, &file_path, &fname, &app_handle, &cancel).await {
            Ok(path) => {
                let progress = DownloadProgress {
                    url: url_clone,
                    filename: fname.clone(),
                    downloaded: 0,
                    total: 0,
                    done: true,
                    path: path.clone(),
                };
                let _ = app_handle.emit("download-progress", &progress);
                println!("[Browser] 下载完成: {}", path);
            }
            Err(e) => {
                eprintln!("[Browser] 下载失败: {}", e);
                let progress = DownloadProgress {
                    url: url_clone,
                    filename: fname,
                    downloaded: 0,
                    total: 0,
                    done: true,
                    path: format!("错误: {}", e),
                };
                let _ = app_handle.emit("download-progress", &progress);
            }
        }
    });

    Ok(format!("开始下载: {}", fname))
}

// ─── 下载引擎 ───

async fn download_file(
    url: &str,
    file_path: &PathBuf,
    filename: &str,
    app: &AppHandle,
    cancel: &AtomicBool,
) -> Result<String, String> {
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目录: {}", e))?;
    }

    let client = Client::builder()
        .user_agent(
            "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36"
        )
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let total_size = response.content_length().unwrap_or(0);
    let mut file = fs::File::create(file_path).map_err(|e| format!("无法创建文件: {}", e))?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            let _ = fs::remove_file(file_path);
            return Err("下载被取消".into());
        }

        let chunk = chunk.map_err(|e| format!("读取数据失败: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("写入文件失败: {}", e))?;
        downloaded += chunk.len() as u64;

        let progress = DownloadProgress {
            url: url.to_string(),
            filename: filename.to_string(),
            downloaded,
            total: total_size,
            done: false,
            path: file_path.to_string_lossy().to_string(),
        };
        let _ = app.emit("download-progress", &progress);
    }

    file.flush().map_err(|e| format!("刷新文件失败: {}", e))?;

    Ok(file_path.to_string_lossy().to_string())
}

// ─── 工具函数 ───

fn get_download_dir(app: &AppHandle) -> Result<PathBuf, String> {
    // 优先使用系统下载目录
    if let Ok(dir) = app.path().download_dir() {
        std::fs::create_dir_all(&dir).ok();
        return Ok(dir);
    }
    // 回退到应用数据目录下的 Downloads
    let fallback = app
        .path()
        .app_data_dir()
        .map(|d| d.join("Downloads"))
        .map_err(|e| format!("无法获取下载目录: {}", e))?;
    std::fs::create_dir_all(&fallback).ok();
    Ok(fallback)
}

fn extract_filename(url: &str, media_type: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("video");

    if name.contains('.') {
        name.to_string()
    } else {
        let ext = match media_type {
            "video" => "mp4",
            "audio" => "mp3",
            "stream" => "mp4",
            "xhr" => "mp4",
            _ => "mp4",
        };
        format!("{}_{}.{}", name, rand_suffix(), ext)
    }
}

fn rand_suffix() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_micros())
        .unwrap_or(0)
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' => '_',
            _ => c,
        })
        .collect()
}

// ─── 应用入口 ───

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(AppState {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })))
        .setup(move |app| {
            // 注入视频检测脚本到主 webview
            if let Some(webview) = app.get_webview("main") {
                let _ = webview.eval(INJECT_SCRIPT);
            }
            println!("[Browser] 应用启动完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            navigate,
            go_home,
            detect_media,
            start_download
        ])
        .run(tauri::generate_context!())
        .expect("启动应用时出错");
}
