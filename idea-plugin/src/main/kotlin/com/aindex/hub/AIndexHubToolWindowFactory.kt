package com.aindex.hub

import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.ui.content.ContentFactory
import com.intellij.ui.jcef.JBCefBrowser
import com.intellij.ui.jcef.JBCefApp
import java.awt.BorderLayout
import javax.swing.JPanel
import javax.swing.JLabel
import javax.swing.SwingConstants

class AIndexHubToolWindowFactory : ToolWindowFactory {

    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val contentFactory = ContentFactory.getInstance()
        
        val panel = JPanel(BorderLayout())
        
        if (!JBCefApp.isSupported()) {
            panel.add(JLabel("JCEF is not supported in this IDEA environment", SwingConstants.CENTER), BorderLayout.CENTER)
            val content = contentFactory.createContent(panel, "", false)
            toolWindow.contentManager.addContent(content)
            return
        }

        // Initialize background bundle if needed
        BundleManager.ensureBundleReady(project) { success ->
            if (success) {
                BundleManager.startBundle { started ->
                    if (started) {
                        val projectPath = project.basePath ?: ""
                        val encodedPath = java.net.URLEncoder.encode(projectPath, "UTF-8")
                        val browser = JBCefBrowser("http://localhost:3000/?ide=idea&project=$encodedPath")
                        panel.add(browser.component, BorderLayout.CENTER)
                    } else {
                        panel.add(JLabel("Failed to start AIndex Hub server", SwingConstants.CENTER), BorderLayout.CENTER)
                    }
                }
            } else {
                panel.add(JLabel("Failed to prepare AIndex Hub environment", SwingConstants.CENTER), BorderLayout.CENTER)
            }
        }

        // Initial loading state
        if (panel.componentCount == 0) {
            panel.add(JLabel("Starting AIndex Hub Environment... (First launch may take time to download files)", SwingConstants.CENTER), BorderLayout.CENTER)
        }

        val content = contentFactory.createContent(panel, "", false)
        toolWindow.contentManager.addContent(content)
    }
}
