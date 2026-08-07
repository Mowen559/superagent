pluginManagement {
    repositories {
        maven { setUrl("https://maven.aliyun.com/repository/gradle-plugin") }
        maven { setUrl("https://maven.aliyun.com/repository/public") }
        gradlePluginPortal()
        mavenCentral()
    }
}
rootProject.name = "aindex-hub-idea-plugin"
