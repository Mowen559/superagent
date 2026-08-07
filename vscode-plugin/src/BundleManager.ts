import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { spawn, ChildProcess } from 'child_process';


export class BundleManager {
  private static instance: BundleManager;
  private bundleProcess: ChildProcess | null = null;
  public readonly bundleDir: string;
  private readonly context: vscode.ExtensionContext;

  constructor(context: vscode.ExtensionContext) {
    BundleManager.instance = this;
    this.context = context;
    // Store bundle in global storage
    this.bundleDir = path.join(context.globalStorageUri.fsPath, 'bundle');
  }

  public static getInstance(): BundleManager {
    return BundleManager.instance;
  }

  public async ensureBundleReady(outputChannel: vscode.OutputChannel): Promise<boolean> {
    if (this.isBundleInstalled()) {
      return true;
    }

    // Scheme B: Integrated Packaging. The bundle is packaged inside the extension at 'bundle' directory.
    const integratedBundlePath = path.join(this.context.extensionPath, 'bundle');
    
    if (fs.existsSync(integratedBundlePath)) {
      outputChannel.appendLine(`Found integrated bundle at ${integratedBundlePath}. Copying to global storage...`);
      await this.copyLocalDevBundle(integratedBundlePath, this.bundleDir);
      outputChannel.appendLine('Runtime installed successfully.');
      return true;
    }

    // Fallback if the user somehow corrupted the package, try dev mode path
    const devBundlePath = path.join(this.context.extensionPath, '..', 'deepcloud-bundle', 'windows-x64');
    if (fs.existsSync(devBundlePath)) {
      outputChannel.appendLine(`Found local dev bundle at ${devBundlePath}. Copying...`);
      await this.copyLocalDevBundle(devBundlePath, this.bundleDir);
      return true;
    }

    vscode.window.showErrorMessage('Integrated AIndex runtime bundle not found in extension package!');
    return false;
  }

  private isBundleInstalled(): boolean {
    return fs.existsSync(path.join(this.bundleDir, 'START-HUB-WEB.cmd'));
  }

  private async copyLocalDevBundle(src: string, dest: string): Promise<void> {
    // Basic recursive copy for dev purposes
    if (!fs.existsSync(dest)) {
      fs.mkdirSync(dest, { recursive: true });
    }
    const entries = fs.readdirSync(src, { withFileTypes: true });
    for (const entry of entries) {
      const srcPath = path.join(src, entry.name);
      const destPath = path.join(dest, entry.name);
      if (entry.isDirectory()) {
        await this.copyLocalDevBundle(srcPath, destPath);
      } else {
        fs.copyFileSync(srcPath, destPath);
      }
    }
  }



  public async startBundle(outputChannel: vscode.OutputChannel): Promise<boolean> {
    if (this.bundleProcess) {
      return true; // Already running
    }
    
    // Check if port 3000 is already in use by checking curl
    try {
      const resp = await fetch('http://localhost:3000/api/health').catch(() => null);
      if (resp && resp.ok) {
        outputChannel.appendLine('Hub is already running externally.');
        return true;
      }
    } catch(e) {}

    const startCmd = path.join(this.bundleDir, 'START-HUB-WEB.cmd');
    if (!fs.existsSync(startCmd)) {
      vscode.window.showErrorMessage('Bundle start script not found.');
      return false;
    }

    outputChannel.appendLine(`Starting bundle process at ${startCmd}`);
    
    this.bundleProcess = spawn('cmd.exe', ['/c', 'START-HUB-WEB.cmd'], {
      cwd: this.bundleDir,
      windowsHide: true
    });

    this.bundleProcess.stdout?.on('data', (data) => {
      outputChannel.appendLine(`[Hub] ${data.toString()}`);
    });

    this.bundleProcess.stderr?.on('data', (data) => {
      outputChannel.appendLine(`[Hub Err] ${data.toString()}`);
    });

    this.bundleProcess.on('close', (code) => {
      outputChannel.appendLine(`Bundle process exited with code ${code}`);
      this.bundleProcess = null;
    });

    // Wait for the server to be ready
    return new Promise((resolve) => {
      let retries = 0;
      const interval = setInterval(async () => {
        try {
          const resp = await fetch('http://localhost:3000').catch(() => null);
          if (resp) {
            clearInterval(interval);
            resolve(true);
          }
        } catch(e) {}
        
        retries++;
        if (retries > 30) {
          clearInterval(interval);
          outputChannel.appendLine('Timeout waiting for bundle to start.');
          resolve(false);
        }
      }, 1000);
    });
  }

  public stopBundle() {
    if (this.bundleProcess) {
      // On Windows, child_process.kill() might not kill the entire process tree. 
      // Using taskkill is more reliable for cmd wrappers.
      spawn('taskkill', ['/pid', this.bundleProcess.pid!.toString(), '/f', '/t']);
      this.bundleProcess = null;
    }
  }
}
