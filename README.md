# 🦀 My Tauri App — Android 版

基于 **Tauri v2** 的跨平台应用，支持 **Android** · Windows · macOS · Linux。

> 所有 Android 编译在 **GitHub Actions** 上完成，本地无需安装 Android SDK。

## 📁 项目结构

```
my_project/
├── .github/workflows/build-android.yml   # CI 自动编译 APK
├── package.json                          # 前端依赖
├── vite.config.js                        # Vite 配置
├── index.html                            # 入口页面
├── src/
│   ├── main.js                           # 前端逻辑 (调用 Rust 命令)
│   └── style.css                         # 样式
├── src-tauri/
│   ├── Cargo.toml                        # Rust 依赖
│   ├── tauri.conf.json                   # Tauri 配置
│   ├── build.rs                          # 构建脚本
│   ├── capabilities/default.json         # 权限声明
│   ├── icons/                            # 应用图标 (CI 自动生成)
│   └── src/
│       ├── main.rs                       # 桌面端入口
│       └── lib.rs                        # 核心逻辑 (桌面 + 移动端共享)
└── README.md
```

## 🚀 快速开始

### 1. 推送到 GitHub

```bash
git init
git add .
git commit -m "初始化 Tauri Android 项目"
git remote add origin https://github.com/你的用户名/仓库名.git
git push -u origin main
```

### 2. 等待 CI 编译

Push 代码后，GitHub Actions 自动开始编译。点击仓库的 **Actions** 标签查看进度。

- ⏱️ 首次编译：约 25–35 分钟
- ⚡ 缓存命中后：约 8–15 分钟

### 3. 下载 APK

1. 打开 GitHub 仓库 → **Actions**
2. 点击最新的 workflow run
3. 在 **Artifacts** 区域下载 `app-debug`
4. 解压后得到 `app-debug.apk`

### 4. 安装到手机

将 APK 传输到 Android 手机，直接安装即可（需要在设置中允许「安装未知来源应用」）。

## 🔧 在本地开发（桌面端）

```bash
# 安装依赖
npm install

# 启动开发服务器 + Tauri 桌面窗口
npm run tauri dev

# 仅构建前端
npm run build
```

> 本地开发只需要 Node.js 和 Rust，不需要 Android SDK。

## 📱 发布到应用商店

### 生成签名密钥

```bash
keytool -genkey -v \
  -keystore release.keystore \
  -alias myapp \
  -keyalg RSA \
  -keysize 2048 \
  -validity 10000
```

### 配置 GitHub Secrets

在 GitHub 仓库 → **Settings** → **Secrets and variables** → **Actions** 中添加：

| Secret | 值 |
|--------|-----|
| `KEYSTORE_BASE64` | `base64 -w0 release.keystore` 的输出 |
| `KEYSTORE_PASSWORD` | 密钥库密码 |
| `KEY_ALIAS` | 密钥别名（如 `myapp`） |
| `KEY_PASSWORD` | 密钥密码 |

然后通过 `workflow_dispatch` 选择 `release` 构建类型即可生成签名的 AAB（用于 Google Play）。

## 🛠️ 技术栈

| 层 | 技术 |
|----|------|
| UI | Vanilla JS + Vite 6 |
| 后端 | Rust + Tauri v2 |
| 移动端 | Android WebView + JNI |
| CI/CD | GitHub Actions |

## 📝 自定义

- **改应用名**：修改 `src-tauri/tauri.conf.json` 中的 `productName`
- **改包名**：修改 `identifier`（如 `com.yourcompany.yourapp`）
- **添加权限**：编辑 `src-tauri/capabilities/default.json`
- **添加 Rust 命令**：在 `src-tauri/src/lib.rs` 中添加 `#[command]` 函数并注册
