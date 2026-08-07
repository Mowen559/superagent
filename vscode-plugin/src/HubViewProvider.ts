import * as vscode from 'vscode';
import { BundleManager } from './BundleManager.js';

export class HubViewProvider implements vscode.WebviewViewProvider {
  public static readonly viewType = 'aindex-hub.webview';
  private _view?: vscode.WebviewView;
  private readonly outputChannel: vscode.OutputChannel;

  constructor(
    private readonly _extensionUri: vscode.Uri,
    private readonly _bundleManager: BundleManager
  ) {
    this.outputChannel = vscode.window.createOutputChannel('AIndex Hub');
  }

  public async resolveWebviewView(
    webviewView: vscode.WebviewView,
    context: vscode.WebviewViewResolveContext,
    _token: vscode.CancellationToken
  ) {
    this._view = webviewView;

    webviewView.webview.options = {
      enableScripts: true,
      localResourceRoots: [this._extensionUri]
    };

    // Show loading state
    webviewView.webview.html = this._getHtmlForLoading();

    // Ensure bundle is ready
    const isReady = await this._bundleManager.ensureBundleReady(this.outputChannel);
    if (!isReady) {
      webviewView.webview.html = this._getHtmlForError('Failed to prepare runtime environment.');
      return;
    }

    // Start bundle
    const isStarted = await this._bundleManager.startBundle(this.outputChannel);
    if (!isStarted) {
      webviewView.webview.html = this._getHtmlForError('Failed to start Hub server.');
      return;
    }

    // Load actual hub
    webviewView.webview.html = this._getHtmlForWebview();
  }

  private _getHtmlForLoading() {
    return `<!DOCTYPE html>
      <html lang="en">
      <head>
        <meta charset="UTF-8">
        <style>
          body { display: flex; justify-content: center; align-items: center; height: 100vh; background: transparent; color: var(--vscode-editor-foreground); font-family: sans-serif; }
          .loader { text-align: center; }
          .spinner { border: 4px solid rgba(255,255,255,0.1); width: 36px; height: 36px; border-radius: 50%; border-left-color: var(--vscode-button-background); animation: spin 1s linear infinite; margin: 0 auto 16px; }
          @keyframes spin { 0% { transform: rotate(0deg); } 100% { transform: rotate(360deg); } }
        </style>
      </head>
      <body>
        <div class="loader">
          <div class="spinner"></div>
          <div>Starting AIndex Hub Environment...</div>
          <div style="font-size: 12px; opacity: 0.6; margin-top: 8px;">First launch may take a minute to copy files.</div>
        </div>
      </body>
      </html>`;
  }

  private _getHtmlForError(message: string) {
    return `<!DOCTYPE html>
      <html lang="en">
      <head>
        <meta charset="UTF-8">
        <style>
          body { padding: 20px; color: var(--vscode-errorForeground); font-family: sans-serif; }
        </style>
      </head>
      <body>
        <h2>Error</h2>
        <p>${message}</p>
        <p>Check the Output panel (AIndex Hub) for details.</p>
      </body>
      </html>`;
  }

  private _getHtmlForWebview() {
    // We use an iframe pointing to localhost:3000
    // To make it look native, we pass some CSS variables if the web app supports them, 
    // or just let it render naturally.
    const workspaceFolders = vscode.workspace.workspaceFolders;
    const projectPath = workspaceFolders && workspaceFolders.length > 0 ? workspaceFolders[0].uri.fsPath : '';
    const encodedPath = encodeURIComponent(projectPath);
    
    return `<!DOCTYPE html>
      <html lang="en">
      <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <style>
          html, body, iframe {
            margin: 0; padding: 0; border: none; width: 100%; height: 100%; overflow: hidden; background: transparent;
          }
        </style>
      </head>
      <body>
        <iframe src="http://localhost:3000/?ide=vscode&project=${encodedPath}" allow="clipboard-read; clipboard-write"></iframe>
      </body>
      </html>`;
  }
}
