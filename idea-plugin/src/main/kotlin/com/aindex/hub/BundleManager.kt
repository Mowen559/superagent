package com.aindex.hub

import com.intellij.openapi.project.Project
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.progress.ProgressIndicator
import com.intellij.openapi.progress.ProgressManager
import com.intellij.openapi.progress.Task
import com.intellij.openapi.application.PathManager
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.execution.process.OSProcessHandler
import com.intellij.execution.process.ProcessAdapter
import com.intellij.execution.process.ProcessEvent
import com.intellij.openapi.util.Key
import java.io.File
import java.net.URL

import java.util.zip.ZipInputStream
import javax.swing.SwingUtilities

object BundleManager {
    private var processHandler: OSProcessHandler? = null
    
    // Use IDEA's global system path
    private val bundleDir = File(PathManager.getSystemPath(), "aindex-bundle")
    
    fun ensureBundleReady(project: Project, callback: (Boolean) -> Unit) {
        val startCmd = File(bundleDir, "START-HUB-WEB.cmd")
        if (startCmd.exists()) {
            callback(true)
            return
        }

        // For local development testing, if bundle is at root of project, we could copy it
        val devBundle = File(project.basePath, "deepcloud-bundle/windows-x64")
        if (devBundle.exists()) {
            ApplicationManager.getApplication().executeOnPooledThread {
                try {
                    devBundle.copyRecursively(bundleDir, overwrite = true)
                    SwingUtilities.invokeLater { callback(true) }
                } catch(e: Exception) {
                    SwingUtilities.invokeLater { callback(false) }
                }
            }
            return
        }

        ProgressManager.getInstance().run(object : Task.Backgroundable(project, "Extracting AIndex Hub Runtime", false) {
            override fun run(indicator: ProgressIndicator) {
                try {
                    indicator.isIndeterminate = true
                    indicator.text = "Extracting integrated bundle..."
                    
                    bundleDir.mkdirs()
                    
                    val bundleStream = javaClass.getResourceAsStream("/bundle.zip")
                    if (bundleStream == null) {
                        throw Exception("Integrated bundle.zip not found in plugin resources!")
                    }

                    ZipInputStream(bundleStream).use { zis ->
                        var entry = zis.nextEntry
                        while (entry != null) {
                            val dest = File(bundleDir, entry.name)
                            if (entry.isDirectory) {
                                dest.mkdirs()
                            } else {
                                dest.parentFile.mkdirs()
                                dest.outputStream().use { out -> zis.copyTo(out) }
                            }
                            entry = zis.nextEntry
                        }
                    }
                    
                    SwingUtilities.invokeLater { callback(true) }
                } catch (e: Exception) {
                    e.printStackTrace()
                    SwingUtilities.invokeLater { callback(false) }
                }
            }
        })
    }

    fun startBundle(callback: (Boolean) -> Unit) {
        if (processHandler != null && !processHandler!!.isProcessTerminated) {
            callback(true)
            return
        }

        // Test if localhost:3000 is already up (e.g. started externally or by VS Code)
        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                val connection = URL("http://localhost:3000/api/health").openConnection()
                connection.connectTimeout = 1000
                connection.readTimeout = 1000
                connection.getInputStream().close()
                SwingUtilities.invokeLater { callback(true) }
                return@executeOnPooledThread
            } catch (e: Exception) {
                // Not running, proceed to start
            }

            try {
                val commandLine = GeneralCommandLine("cmd.exe", "/c", "START-HUB-WEB.cmd")
                commandLine.workDirectory = bundleDir
                
                processHandler = OSProcessHandler(commandLine)
                processHandler!!.addProcessListener(object : ProcessAdapter() {
                    override fun onTextAvailable(event: ProcessEvent, outputType: Key<*>) {
                        // Print to IDEA log if needed
                        println("[AIndex Hub] ${event.text}")
                    }
                })
                processHandler!!.startNotify()

                // Poll for readiness
                var retries = 0
                while (retries < 30) {
                    try {
                        val connection = URL("http://localhost:3000/").openConnection()
                        connection.connectTimeout = 1000
                        connection.readTimeout = 1000
                        connection.getInputStream().close()
                        SwingUtilities.invokeLater { callback(true) }
                        return@executeOnPooledThread
                    } catch (e: Exception) {
                        Thread.sleep(1000)
                        retries++
                    }
                }
                SwingUtilities.invokeLater { callback(false) }
            } catch (e: Exception) {
                e.printStackTrace()
                SwingUtilities.invokeLater { callback(false) }
            }
        }
    }
}
