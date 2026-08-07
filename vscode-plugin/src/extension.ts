import * as vscode from 'vscode';
import { HubViewProvider } from './HubViewProvider.js';
import { BundleManager } from './BundleManager.js';

export async function activate(context: vscode.ExtensionContext) {
  console.log('AIndex Hub extension is now active!');

  const bundleManager = new BundleManager(context);
  
  // Register Webview View
  const provider = new HubViewProvider(context.extensionUri, bundleManager);
  context.subscriptions.push(
    vscode.window.registerWebviewViewProvider(HubViewProvider.viewType, provider)
  );

  // Command to open webview manually or trigger start
  let disposable = vscode.commands.registerCommand('aindex-hub.start', async () => {
    vscode.commands.executeCommand('workbench.view.extension.aindex-hub');
  });

  context.subscriptions.push(disposable);
}

export function deactivate() {
  BundleManager.getInstance()?.stopBundle();
}
