plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "dev.weiyi.glance.remote"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.weiyi.glance.remote"
        minSdk = 26
        targetSdk = 35
        versionCode = 17
        versionName = "0.4.3"
    }

    // The key the app is published under is its owner's alone: kept outside
    // this repository, said where by GLANCE_APK_KEYSTORE, its password by
    // GLANCE_APK_PASSWORD (the key in it is named "glance"). Without them
    // the release is built unsigned, as anyone can build it.
    val keystore = System.getenv("GLANCE_APK_KEYSTORE")
    val password = System.getenv("GLANCE_APK_PASSWORD")
    val published = if (keystore != null && password != null) {
        signingConfigs.create("published") {
            storeFile = file(keystore)
            storePassword = password
            keyAlias = "glance"
            keyPassword = password
        }
    } else {
        null
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            signingConfig = published
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    // The pairing code's QR, read without Google's services (phones sold
    // in mainland China have none).
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
}
