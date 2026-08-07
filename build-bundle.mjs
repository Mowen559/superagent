#!/usr/bin/env node

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { spawn } from "child_process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const repoRoot = __dirname;
const hubWebRoot = path.join(repoRoot, "hub-web");
const codebaseMemoryRoot = path.join(repoRoot, "codebase-memory-mcp");
const supermemoryRoot = path.join(repoRoot, "supermemory");

const args = parseArgs(process.argv.slice(2));
const platform = normalizePlatform(args.platform || process.platform);
const arch = normalizeArch(args.arch || process.arch);
const bundleDir = path.resolve(args.output || path.join(repoRoot, "deepcloud-bundle", `${platform}-${arch}`));
const skipBuild = args["skip-build"] === true;
const skipSupermemory = args["skip-supermemory"] === true;
const doInstall = args.install === true;
const hostPlatform = normalizePlatform(process.platform);
const hostArch = normalizeArch(process.arch);

async function main() {
  if (platform !== hostPlatform || arch !== hostArch) {
    throw new Error(
      `Cross-platform packaging is not supported from this host. Host is ${hostPlatform}-${hostArch}, requested ${platform}-${arch}. Run the script on the target OS/CPU instead.`,
    );
  }

  logBanner(`Building DeepCloud bundle for ${platform}-${arch}`);

  ensurePathExists(hubWebRoot, "hub-web");
  ensurePathExists(codebaseMemoryRoot, "codebase-memory-mcp");

  if (doInstall) {
    await runCommand("pnpm", ["install"], { cwd: repoRoot });
  }

  if (!skipBuild) {
    await runCommand("npm", ["run", "build"], { cwd: hubWebRoot });
  }

  const standaloneRoot = path.join(hubWebRoot, ".next", "standalone");
  const standaloneAppRoot = path.join(standaloneRoot, "hub-web");
  const staticRoot = path.join(hubWebRoot, ".next", "static");
  const publicRoot = path.join(hubWebRoot, "public");
  const hubWebNodeModules = path.join(hubWebRoot, "node_modules");

  ensurePathExists(standaloneRoot, "Next standalone output");
  ensurePathExists(standaloneAppRoot, "standalone hub-web app");
  ensurePathExists(staticRoot, "Next static output");
  ensurePathExists(hubWebNodeModules, "hub-web node_modules");

  resetDir(bundleDir);
  const webDir = path.join(bundleDir, "web");
  const runtimeDir = path.join(bundleDir, "runtime");
  const nodeRuntimeDir = path.join(runtimeDir, "node");
  const toolsDir = path.join(bundleDir, "mcp-tools");
  const supermemoryBundleDir = path.join(toolsDir, "supermemory-mcp");

  mkdirp(webDir);
  mkdirp(runtimeDir);
  mkdirp(toolsDir);

  logStep("Copying standalone web bundle");
  copyDir(standaloneRoot, webDir);
  copyDir(staticRoot, path.join(webDir, "hub-web", ".next", "static"));
  if (fs.existsSync(publicRoot)) {
    copyDir(publicRoot, path.join(webDir, "hub-web", "public"));
  }
  const localesRoot = path.join(hubWebRoot, "src", "locales");
  if (fs.existsSync(localesRoot)) {
    copyDir(localesRoot, path.join(webDir, "hub-web", "src", "locales"));
  }
  logStep("Flattening portable node_modules");
  flattenPnpmStandalone(webDir);

  logStep("Embedding Node runtime");
  embedNodeRuntime(nodeRuntimeDir, platform);

  logStep("Installing codebase-memory-mcp into bundle");
  await installCodebaseMemory(path.join(toolsDir, "codebase-memory-mcp"));

  if (!skipSupermemory && fs.existsSync(supermemoryRoot)) {
    logStep("Copying supermemory source bundle");
    copyDir(supermemoryRoot, supermemoryBundleDir, {
      excludeNames: new Set([".git", "node_modules", ".turbo", "dist"]),
    });
  }

  logStep("Generating portable launchers and docs");
  writeLaunchers(bundleDir, platform);
  writeBundleDocs(bundleDir, {
    platform,
    arch,
    includesSupermemory: !skipSupermemory && fs.existsSync(supermemoryRoot),
  });
  writeBundleManifest(bundleDir, { platform, arch });

  verifyBundle(bundleDir, platform);

  logBanner(`Bundle ready: ${bundleDir}`);
}

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) continue;

    const trimmed = token.slice(2);
    const [rawKey, rawValue] = trimmed.split("=", 2);
    if (rawValue !== undefined) {
      parsed[rawKey] = rawValue;
      continue;
    }

    const next = argv[index + 1];
    if (next && !next.startsWith("--")) {
      parsed[rawKey] = next;
      index += 1;
    } else {
      parsed[rawKey] = true;
    }
  }
  return parsed;
}

function normalizePlatform(value) {
  const normalized = String(value).toLowerCase();
  if (normalized === "win32" || normalized === "windows") return "windows";
  if (normalized === "darwin" || normalized === "mac" || normalized === "macos") return "macos";
  if (normalized === "linux") return "linux";
  throw new Error(`Unsupported platform: ${value}`);
}

function normalizeArch(value) {
  const normalized = String(value).toLowerCase();
  if (normalized === "x64" || normalized === "amd64") return "x64";
  if (normalized === "arm64" || normalized === "aarch64") return "arm64";
  throw new Error(`Unsupported architecture: ${value}`);
}

function ensurePathExists(targetPath, label) {
  if (!fs.existsSync(targetPath)) {
    throw new Error(`Missing ${label}: ${targetPath}`);
  }
}

function mkdirp(targetPath) {
  fs.mkdirSync(targetPath, { recursive: true });
}

function resetDir(targetPath) {
  fs.rmSync(targetPath, { recursive: true, force: true });
  mkdirp(targetPath);
}

function copyDir(source, destination, options = {}) {
  const excludeNames = new Set([".git", ...(options.excludeNames || new Set())]);
  mkdirp(destination);

  for (const entry of fs.readdirSync(source, { withFileTypes: true })) {
    if (excludeNames.has(entry.name)) {
      continue;
    }

    const sourcePath = path.join(source, entry.name);
    const destinationPath = path.join(destination, entry.name);
    
    // Skip symbolic links completely to avoid massive duplication from Next.js standalone pnpm trace
    if (entry.isSymbolicLink()) {
      continue;
    }
    
    const resolvedSourcePath = sourcePath;
    const stats = fs.statSync(resolvedSourcePath);

    if (stats.isDirectory()) {
      copyDir(resolvedSourcePath, destinationPath, options);
      continue;
    }

    mkdirp(path.dirname(destinationPath));
    fs.copyFileSync(resolvedSourcePath, destinationPath);
  }
}

function embedNodeRuntime(destinationRoot, targetPlatform) {
  resetDir(destinationRoot);

  const currentNode = process.execPath;
  ensurePathExists(currentNode, "current Node runtime");

  if (targetPlatform === "windows") {
    fs.copyFileSync(currentNode, path.join(destinationRoot, "node.exe"));
    return;
  }

  const binDir = path.join(destinationRoot, "bin");
  mkdirp(binDir);
  const destinationBinary = path.join(binDir, "node");
  fs.copyFileSync(currentNode, destinationBinary);
  fs.chmodSync(destinationBinary, 0o755);
}

async function installCodebaseMemory(destination) {
  resetDir(destination);

  if (process.platform === "win32") {
    const installer = path.join(codebaseMemoryRoot, "install.ps1");
    await runCommand("powershell.exe", [
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      installer,
      `--dir=${destination}`,
      "--skip-config",
    ], {
      cwd: codebaseMemoryRoot,
    });
    const currentUserPath = await getWindowsUserPath();
    await setWindowsUserPath(sanitizeWindowsUserPath(currentUserPath, destination));
    return;
  }

  const installer = path.join(codebaseMemoryRoot, "install.sh");
  await runCommand("bash", [installer, `--dir=${destination}`, "--skip-config"], {
    cwd: codebaseMemoryRoot,
  });
}

async function getWindowsUserPath() {
  if (process.platform !== "win32") return "";
  return captureCommand("powershell.exe", [
    "-NoProfile",
    "-Command",
    "[Environment]::GetEnvironmentVariable('PATH', 'User')",
  ]);
}

async function setWindowsUserPath(nextPath) {
  if (process.platform !== "win32") return;
  await runCommand("powershell.exe", [
    "-NoProfile",
    "-Command",
    `[Environment]::SetEnvironmentVariable('PATH', @'\n${nextPath}\n'@, 'User')`,
  ], {
    cwd: repoRoot,
  });
}

function sanitizeWindowsUserPath(currentPath, destination) {
  const normalizedDestination = normalizePathForCompare(destination);
  return currentPath
    .split(";")
    .map((entry) => entry.trim())
    .filter(Boolean)
    .filter((entry) => {
      const normalizedEntry = normalizePathForCompare(entry);
      if (!normalizedEntry) {
        return false;
      }

      if (normalizedEntry === normalizedDestination) {
        return false;
      }

      return !normalizedEntry.includes("\\deepcloud-bundle\\") ||
        !normalizedEntry.endsWith("\\mcp-tools\\codebase-memory-mcp");
    })
    .join(";");
}

function normalizePathForCompare(targetPath) {
  try {
    return path.resolve(targetPath).replace(/\//g, "\\").toLowerCase();
  } catch {
    return targetPath.replace(/\//g, "\\").toLowerCase();
  }
}

function writeLaunchers(bundleRoot, targetPlatform) {
  const isWindows = targetPlatform === "windows";
  const cmdLauncher = `@echo off
setlocal
for %%I in ("%~dp0.") do set "APP_ROOT=%%~fI"
if "%LOCALAPPDATA%"=="" (
  set "HUB_APP_DATA_ROOT=%USERPROFILE%\\AppData\\Local\\HubWeb"
) else (
  set "HUB_APP_DATA_ROOT=%LOCALAPPDATA%\\HubWeb"
)
set "HUB_BUNDLE_ROOT=%APP_ROOT%"
set "HUB_WORKSPACE_ROOT=%APP_ROOT%\\web"
set "HUB_GITNEXUS_ROOT=%APP_ROOT%\\web\\GitNexus"
set "HUB_GITNEXUS_REPO_ROOT=%APP_ROOT%\\web\\GitNexus\\gitnexus"
set "HUB_UNDERSTAND_ANYTHING_ROOT=%APP_ROOT%\\web\\Understand-Anything\\understand-anything-plugin"
set "HUB_CODEGRAPH_ROOT=%APP_ROOT%\\web\\codegraph"
set "HUB_PROJECTS_ROOT=%HUB_APP_DATA_ROOT%\\projects"
set "HUB_SHADOW_GIT_ROOT=%HUB_APP_DATA_ROOT%\\shadow-git"
set "HUB_CODEBASE_MEMORY_CACHE_ROOT=%HUB_APP_DATA_ROOT%\\cache\\codebase-memory-mcp"
set "HUB_CODEBASE_MEMORY_BIN=%APP_ROOT%\\mcp-tools\\codebase-memory-mcp\\codebase-memory-mcp.exe"
set "HOSTNAME=0.0.0.0"
if not exist "%HUB_APP_DATA_ROOT%" mkdir "%HUB_APP_DATA_ROOT%"
"%APP_ROOT%\\runtime\\node\\node.exe" "%APP_ROOT%\\web\\hub-web\\server.js"
`;

  const unixLauncher = `#!/usr/bin/env bash
set -euo pipefail

APP_ROOT="$(cd "$(dirname "$0")" && pwd)"

if [ -z "\${HUB_APP_DATA_ROOT:-}" ]; then
  case "$(uname -s)" in
    Darwin)
      HUB_APP_DATA_ROOT="$HOME/Library/Application Support/HubWeb"
      ;;
    *)
      HUB_APP_DATA_ROOT="\${XDG_DATA_HOME:-$HOME/.local/share}/hub-web"
      ;;
  esac
fi

export HUB_APP_DATA_ROOT
export HUB_BUNDLE_ROOT="$APP_ROOT"
export HUB_WORKSPACE_ROOT="$APP_ROOT/web"
export HUB_GITNEXUS_ROOT="$APP_ROOT/web/GitNexus"
export HUB_GITNEXUS_REPO_ROOT="$APP_ROOT/web/GitNexus/gitnexus"
export HUB_UNDERSTAND_ANYTHING_ROOT="$APP_ROOT/web/Understand-Anything/understand-anything-plugin"
export HUB_CODEGRAPH_ROOT="$APP_ROOT/web/codegraph"
export HUB_PROJECTS_ROOT="$HUB_APP_DATA_ROOT/projects"
export HUB_SHADOW_GIT_ROOT="$HUB_APP_DATA_ROOT/shadow-git"
export HUB_CODEBASE_MEMORY_CACHE_ROOT="$HUB_APP_DATA_ROOT/cache/codebase-memory-mcp"
export HUB_CODEBASE_MEMORY_BIN="$APP_ROOT/mcp-tools/codebase-memory-mcp/codebase-memory-mcp"
export HOSTNAME="0.0.0.0"

mkdir -p "$HUB_APP_DATA_ROOT"
exec "$APP_ROOT/runtime/node/bin/node" "$APP_ROOT/web/hub-web/server.js"
`;

  fs.writeFileSync(path.join(bundleRoot, "START-HUB-WEB.cmd"), cmdLauncher, "utf8");
  fs.writeFileSync(path.join(bundleRoot, "START-HUB-WEB.sh"), unixLauncher, "utf8");
  fs.writeFileSync(path.join(bundleRoot, "START-HUB-WEB.command"), unixLauncher, "utf8");
  fs.chmodSync(path.join(bundleRoot, "START-HUB-WEB.sh"), 0o755);
  fs.chmodSync(path.join(bundleRoot, "START-HUB-WEB.command"), 0o755);

  if (isWindows) {
    fs.writeFileSync(path.join(bundleRoot, "START-ALL.bat"), cmdLauncher, "utf8");
    return;
  }

  fs.writeFileSync(path.join(bundleRoot, "START-ALL.sh"), unixLauncher, "utf8");
  fs.chmodSync(path.join(bundleRoot, "START-ALL.sh"), 0o755);
}

function writeBundleDocs(bundleRoot, { platform, arch, includesSupermemory }) {
  const runtimeBinary =
    platform === "windows"
      ? "runtime\\node\\node.exe"
      : "runtime/node/bin/node";
  const codebaseBinary =
    platform === "windows"
      ? "mcp-tools\\codebase-memory-mcp\\codebase-memory-mcp.exe"
      : "mcp-tools/codebase-memory-mcp/codebase-memory-mcp";

  const readme = `# DeepCloud Portable Bundle

This directory is ready to be wrapped by install4j, zip, dmg, or tar.

## Included

- Standalone hub-web server bundle
- Embedded Node runtime (${runtimeBinary})
- Embedded codebase-memory-mcp binary (${codebaseBinary})
${includesSupermemory ? "- Supermemory source bundle for optional MCP integration" : "- Supermemory source bundle was skipped"}

## Start locally

- Windows: run START-HUB-WEB.cmd
- macOS: run START-HUB-WEB.command
- Linux: run ./START-HUB-WEB.sh

## Persistent data

The app stores mutable data outside the install directory:

- Windows: %LOCALAPPDATA%\\HubWeb
- macOS: ~/Library/Application Support/HubWeb
- Linux: ~/.local/share/hub-web

## install4j packaging

Point install4j at this bundle directory and use:

- Windows launcher: START-HUB-WEB.cmd
- macOS launcher: START-HUB-WEB.command
- Linux launcher: START-HUB-WEB.sh

If you prefer a direct executable launcher, target:

- Working directory: web/hub-web
- Executable: ${runtimeBinary}
- Argument: web/hub-web/server.js

The launcher must preserve the environment variables configured by the startup script.
`;

  const install4jNotes = `install4j notes for ${platform}-${arch}

1. Application home directory should be the root of this bundle.
2. Use START-HUB-WEB.* as the launcher entry for the current platform.
3. Keep the bundle layout unchanged:
   - web/
   - runtime/
   - mcp-tools/
4. Do not install writable app data inside the application directory.
5. If you create a native launcher instead of using the script, copy the environment variables from the startup script.
`;

  fs.writeFileSync(path.join(bundleRoot, "README.md"), readme, "utf8");
  fs.writeFileSync(path.join(bundleRoot, "INSTALL4J.txt"), install4jNotes, "utf8");
}

function writeBundleManifest(bundleRoot, { platform, arch }) {
  const manifest = {
    name: "deepcloud-bundle",
    builtAt: new Date().toISOString(),
    builtOn: {
      platform: process.platform,
      arch: process.arch,
      node: process.version,
    },
    target: {
      platform,
      arch,
    },
  };

  fs.writeFileSync(
    path.join(bundleRoot, "bundle-manifest.json"),
    JSON.stringify(manifest, null, 2),
    "utf8",
  );
}

function verifyBundle(bundleRoot, targetPlatform) {
  const checks = [
    path.join(bundleRoot, "web", "hub-web", "server.js"),
    path.join(bundleRoot, "web", "hub-web", ".next", "static"),
    path.join(bundleRoot, "README.md"),
    path.join(bundleRoot, "INSTALL4J.txt"),
  ];

  if (targetPlatform === "windows") {
    checks.push(
      path.join(bundleRoot, "runtime", "node", "node.exe"),
      path.join(bundleRoot, "mcp-tools", "codebase-memory-mcp", "codebase-memory-mcp.exe"),
      path.join(bundleRoot, "START-HUB-WEB.cmd"),
    );
  } else {
    checks.push(
      path.join(bundleRoot, "runtime", "node", "bin", "node"),
      path.join(bundleRoot, "mcp-tools", "codebase-memory-mcp", "codebase-memory-mcp"),
      path.join(bundleRoot, "START-HUB-WEB.sh"),
    );
  }

  for (const target of checks) {
    ensurePathExists(target, "bundle artifact");
  }
}

function logStep(message) {
  console.log(`\n[build-bundle] ${message}`);
}

function logBanner(message) {
  console.log("\n========================================================");
  console.log(message);
  console.log("========================================================");
}

async function runCommand(command, commandArgs, options) {
  const resolvedCommand = process.platform === "win32" && (command === "npm" || command === "pnpm")
    ? `${command}.cmd`
    : command;

  await new Promise((resolve, reject) => {
    const useShell = process.platform === "win32" && (resolvedCommand.endsWith(".cmd") || resolvedCommand.endsWith(".bat"));
    const child = spawn(resolvedCommand, commandArgs, {
      cwd: options.cwd,
      stdio: "inherit",
      shell: useShell,
      env: process.env,
    });

    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) {
        resolve();
        return;
      }

      reject(new Error(`${resolvedCommand} exited with code ${code}`));
    });
  });
}

async function captureCommand(command, commandArgs) {
  const resolvedCommand = process.platform === "win32" && (command === "npm" || command === "pnpm")
    ? `${command}.cmd`
    : command;

  return new Promise((resolve, reject) => {
    let stdout = "";
    let stderr = "";
    const useShell = process.platform === "win32" && (resolvedCommand.endsWith(".cmd") || resolvedCommand.endsWith(".bat"));
    const child = spawn(resolvedCommand, commandArgs, {
      cwd: repoRoot,
      stdio: ["ignore", "pipe", "pipe"],
      shell: useShell,
      env: process.env,
    });

    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) {
        resolve(stdout.trim());
        return;
      }
      reject(new Error(stderr.trim() || `${resolvedCommand} exited with code ${code}`));
    });
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

function flattenPnpmStandalone(webDir) {
  const standaloneNodeModules = path.join(webDir, "node_modules");
  const standaloneHubWebNodeModules = path.join(webDir, "hub-web", "node_modules");

  // 1. Delete absolute junctions that Next.js creates in standalone/hub-web/node_modules
  if (fs.existsSync(standaloneHubWebNodeModules)) {
    const items = fs.readdirSync(standaloneHubWebNodeModules);
    for (const item of items) {
      const p = path.join(standaloneHubWebNodeModules, item);
      if (fs.lstatSync(p).isSymbolicLink()) {
        fs.rmSync(p, { force: true });
      }
    }
  }

  // 2. Flatten all packages from .pnpm into standalone/node_modules
  const pnpmDir = path.join(standaloneNodeModules, ".pnpm");
  if (fs.existsSync(pnpmDir)) {
    const packages = fs.readdirSync(pnpmDir);
    for (const pkg of packages) {
      if (pkg.startsWith(".")) continue;
      
      const innerNodeModules = path.join(pnpmDir, pkg, "node_modules");
      if (!fs.existsSync(innerNodeModules)) continue;
      
      const copyInner = (srcDir, destDir) => {
        const items = fs.readdirSync(srcDir);
        for (const item of items) {
          const srcPath = path.join(srcDir, item);
          const destPath = path.join(destDir, item);
          
          const stat = fs.lstatSync(srcPath);
          if (stat.isSymbolicLink()) {
            // Skip symlinks; Node.js will find them at the root outModulesDir
            continue;
          }
          
          if (stat.isDirectory() && item.startsWith("@")) {
            // It's a scope, recurse
            copyInner(srcPath, destPath);
          } else if (stat.isDirectory()) {
            if (!fs.existsSync(destPath)) {
              fs.cpSync(srcPath, destPath, { recursive: true });
            }
          } else {
            if (!fs.existsSync(destPath)) {
              fs.cpSync(srcPath, destPath);
            }
          }
        }
      };
      copyInner(innerNodeModules, standaloneNodeModules);
    }
    // Delete .pnpm after flattening to save space and remove remaining symlinks/junctions
    fs.rmSync(pnpmDir, { recursive: true, force: true });
  }
}
