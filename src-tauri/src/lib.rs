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

// ─── JS 注入脚本（每页加载后运行） ───

const INJECT_SCRIPT: &str = r#"
(function(){
if(window.__browserInjected)return;window.__browserInjected=1;

// ── 状态栏安全区域 ──
let sb=document.createElement('div');
sb.id='__sb';
sb.style.cssText='position:fixed;top:0;left:0;right:0;height:36px;z-index:2147483646;pointer-events:none;';
document.documentElement.appendChild(sb);
document.body.style.paddingTop='36px';

// ── Scheme 拦截（location.href / assign / replace / window.open / <a> 点击） ──
let ALLOWED=/^(http|https|javascript|mailto|tel|about|data|blob|#|asset|tauri):/i;
function blocked(u){
  if(!u||typeof u!=='string')return false;
  let s=u.trim().split(':')[0];
  return s&&!ALLOWED.test(s+':');
}
// 拦截 location.href setter
try{
  let d=Object.getOwnPropertyDescriptor(window.HTMLAnchorElement?window.HTMLAnchorElement.prototype:window.location.constructor.prototype,'href')||Object.getOwnPropertyDescriptor(Location.prototype,'href');
  if(d&&d.set){
    let _s=d.set;
    Object.defineProperty(Location.prototype,'href',{get:d.get,set:function(v){if(blocked(v)){console.log('[Block href]',v.split(':')[0]);return;}_s.call(this,v);},configurable:true});
  }
}catch(e){console.log('href hook err:',e);}
// 拦截 location.assign
try{
  let _a=Location.prototype.assign;Location.prototype.assign=function(v){if(blocked(v)){console.log('[Block assign]',v.split(':')[0]);return;}_a.call(this,v);};
}catch(e){}
// 拦截 location.replace
try{let _r=Location.prototype.replace;Location.prototype.replace=function(v){if(blocked(v)){console.log('[Block replace]',v.split(':')[0]);return;}_r.call(this,v);};}catch(e){}
// 拦截 window.open
(function(){let _ow=window.open;window.open=function(u){if(typeof u==='string'&&blocked(u))return null;return _ow.apply(this,arguments);};})();
// 拦截 <a> 点击
document.addEventListener('click',function(e){
  let a=e.target.closest('a');if(!a||!a.href)return;
  let raw=a.getAttribute('href')||'';if(!raw||raw.startsWith('#'))return;
  if(blocked(a.href)){e.preventDefault();e.stopImmediatePropagation();console.log('[Block click]',a.href.split(':')[0]);}
},true);

// ── 视频检测 ──
let detected=new Set();
function notify(url,type,title){
  let k=url+'|'+type;if(detected.has(k))return;detected.add(k);
  try{window.__TAURI__&&window.__TAURI__.invoke('detect_media',{url:url,mediaType:type,pageTitle:document.title||''});}catch(e){}
}
function scan(){
  document.querySelectorAll('video,audio').forEach(function(el){
    if(el.src&&el.src.startsWith('http'))notify(el.src,el.tagName.toLowerCase(),document.title);
    el.querySelectorAll('source').forEach(function(s){if(s.src&&s.src.startsWith('http'))notify(s.src,el.tagName.toLowerCase(),document.title);});
  });
}
scan();new MutationObserver(scan).observe(document.documentElement,{childList:true,subtree:true});
// 拦截网络请求
let _fetch=window.fetch;window.fetch=function(url,opts){let u=typeof url==='string'?url:(url instanceof Request?url.url:'');if(/\.(m3u8|mp4|ts|m4v|mkv|webm|flv)(\?|$)/i.test(u))notify(u,'stream',document.title);return _fetch.apply(this,arguments);};
let _xhr=XMLHttpRequest.prototype.open;XMLHttpRequest.prototype.open=function(m,u){if(/\.(m3u8|mp4|ts|m4v|mkv|webm|flv)(\?|$)/i.test(u))notify(u,'xhr',document.title);return _xhr.apply(this,arguments);};

// ── 浮动按钮（右下角） ──
let ui=document.createElement('div');ui.id='__browser_float';
ui.innerHTML='<button id="__b_home" title="返回">🏠</button><button id="__b_dl" style="display:none" title="下载"></button>';
ui.style.cssText='position:fixed;bottom:20px;right:20px;z-index:2147483647;display:flex;flex-direction:column;gap:8px;font-size:24px;';
document.body.appendChild(ui);
document.getElementById('__b_home').onclick=function(){try{window.__TAURI__&&window.__TAURI__.invoke('go_home');}catch(e){}};

try{
  window.__TAURI__&&window.__TAURI__.event.listen('media-detected',function(ev){
    let p=ev.payload,btn=document.getElementById('__b_dl');
    btn.style.display='block';btn.textContent='⬇ '+p.filename+' ('+p.mediaType+')';btn.title=p.url;
    btn.onclick=function(){try{window.__TAURI__&&window.__TAURI__.invoke('start_download',{url:p.url,filename:p.filename});}catch(e){}btn.textContent='⏳ 下载中...';};
  });
  window.__TAURI__&&window.__TAURI__.event.listen('download-progress',function(ev){
    let p=ev.payload,btn=document.getElementById('__b_dl');
    if(p.done){btn.textContent='✅ 完成';btn.style.display='block';}else{let mb=(p.downloaded/1048576).toFixed(1),t=p.total>0?(p.total/1048576).toFixed(1):'?';btn.textContent='⏳ '+mb+'/'+t+' MB';}
  });
}catch(e){}
console.log('[Browser] injected');
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

// ─── 命令 ───

#[command]
async fn navigate(app: AppHandle, url: String) -> Result<(), String> {
    let webview = app.get_webview_window("main").ok_or("未找到主窗口")?;
    let target_url = if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else if url.contains('.') && !url.contains(' ') {
        format!("https://{}", url)
    } else {
        format!("https://www.google.com/search?q={}", url.replace(' ', "+"))
    };

    let parsed = Url::parse(&target_url).map_err(|e| format!("无效的 URL: {}", e))?;
    webview.navigate(parsed).map_err(|e| format!("导航失败: {}", e))?;

    // 页面加载后立即注入脚本（1秒内）
    let wv = webview.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let _ = wv.eval(INJECT_SCRIPT);
    });

    Ok(())
}

#[command]
async fn go_home(app: AppHandle) -> Result<(), String> {
    let webview = app.get_webview_window("main").ok_or("未找到主窗口")?;
    let _ = webview.eval("location.href = '/'");
    Ok(())
}

#[command]
async fn detect_media(
    app: AppHandle, url: String, media_type: String, page_title: String,
) -> Result<(), String> {
    let filename = extract_filename(&url, &media_type);
    let info = MediaInfo { url: url.clone(), media_type, page_title, filename: filename.clone(), size: "未知".into() };
    app.emit("media-detected", &info).map_err(|e| format!("事件发送失败: {}", e))?;
    println!("[Browser] 检测到媒体: {} ({})", filename, url);
    Ok(())
}

#[command]
async fn start_download(
    app: AppHandle,
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
    url: String, filename: String,
) -> Result<String, String> {
    let state = state.lock().await;
    state.cancel_flag.store(false, Ordering::SeqCst);
    let cancel = state.cancel_flag.clone();
    drop(state);

    let download_dir = get_download_dir(&app)?;
    let fname = sanitize_filename(&filename);
    let file_path = download_dir.join(&fname);

    let app_handle = app.clone();
    let url_clone = url.clone();
    let f_dl = fname.clone();
    let f_ok = fname.clone();
    let f_err = fname.clone();

    tokio::spawn(async move {
        match download_file(&url_clone, &file_path, &f_dl, &app_handle, &cancel).await {
            Ok(path) => {
                let _ = app_handle.emit("download-progress", &DownloadProgress {
                    url: url_clone, filename: f_ok, downloaded: 0, total: 0, done: true, path,
                });
            }
            Err(e) => {
                let _ = app_handle.emit("download-progress", &DownloadProgress {
                    url: url_clone, filename: f_err, downloaded: 0, total: 0, done: true,
                    path: format!("错误: {}", e),
                });
            }
        }
    });

    Ok(format!("开始下载: {}", fname))
}

async fn download_file(
    url: &str, file_path: &PathBuf, filename: &str, app: &AppHandle, cancel: &AtomicBool,
) -> Result<String, String> {
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目录: {}", e))?;
    }
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36")
        .build().map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let response = client.get(url).send().await.map_err(|e| format!("请求失败: {}", e))?;
    let total_size = response.content_length().unwrap_or(0);
    let mut file = fs::File::create(file_path).map_err(|e| format!("无法创建文件: {}", e))?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::SeqCst) { let _ = fs::remove_file(file_path); return Err("下载被取消".into()); }
        let chunk = chunk.map_err(|e| format!("读取数据失败: {}", e))?;
        file.write_all(&chunk).map_err(|e| format!("写入文件失败: {}", e))?;
        downloaded += chunk.len() as u64;
        let _ = app.emit("download-progress", &DownloadProgress {
            url: url.to_string(), filename: filename.to_string(), downloaded, total: total_size,
            done: false, path: file_path.to_string_lossy().to_string(),
        });
    }
    file.flush().map_err(|e| format!("刷新文件失败: {}", e))?;
    Ok(file_path.to_string_lossy().to_string())
}

fn get_download_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(dir) = app.path().download_dir() { fs::create_dir_all(&dir).ok(); return Ok(dir); }
    let fallback = app.path().app_data_dir().map(|d| d.join("Downloads")).map_err(|e| format!("{}", e))?;
    fs::create_dir_all(&fallback).ok();
    Ok(fallback)
}

fn extract_filename(url: &str, media_type: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("video");
    if name.contains('.') { name.to_string() } else {
        let ext = match media_type { "video" => "mp4", "audio" => "mp3", _ => "mp4" };
        format!("{}_{}.{}", name, rand_suffix(), ext)
    }
}

fn rand_suffix() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_micros()).unwrap_or(0)
}

fn sanitize_filename(name: &str) -> String {
    name.chars().map(|c| match c { '/'|'\\'|':'|'*'|'?'|'"'|'<'|'>'|'|'|'\n'|'\r' => '_', _ => c }).collect()
}

// ─── 应用入口 ───

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(AppState { cancel_flag: Arc::new(AtomicBool::new(false)) })))
        .setup(move |app| {
            if let Some(webview) = app.get_webview_window("main") {
                let _ = webview.eval(INJECT_SCRIPT);
            }
            println!("[Browser] 应用启动完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![navigate, go_home, detect_media, start_download])
        .run(tauri::generate_context!())
        .expect("启动应用时出错");
}
