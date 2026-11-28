package com.cranibox.android.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

private val LightColors = lightColorScheme(
    primary = Color(0xFF05603A),
    onPrimary = Color.White,
    primaryContainer = Color(0xFF8BF0B6),
    onPrimaryContainer = Color(0xFF002111),
    secondary = Color(0xFF4F6354),
    secondaryContainer = Color(0xFFD1E8D5),
    tertiary = Color(0xFF3B6470),
    error = Color(0xFFBA1A1A),
    background = Color(0xFFF6FBF4),
    surface = Color(0xFFF6FBF4),
    surfaceContainer = Color(0xFFEAEFE8),
    surfaceContainerHigh = Color(0xFFE4E9E2),
)
private val DarkColors = darkColorScheme(
    primary = Color(0xFF6FD49B),
    onPrimary = Color(0xFF00391F),
    primaryContainer = Color(0xFF004F2C),
    onPrimaryContainer = Color(0xFF8BF0B6),
    secondary = Color(0xFFB5CCB9),
    secondaryContainer = Color(0xFF384B3D),
    tertiary = Color(0xFFA3CDD9),
    error = Color(0xFFFFB4AB),
    background = Color(0xFF101510),
    surface = Color(0xFF101510),
    surfaceContainer = Color(0xFF1C211C),
    surfaceContainerHigh = Color(0xFF272B26),
)

@Composable
fun CraniboxTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val colors = if (Build.VERSION.SDK_INT >= 31) {
        if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
    } else if (dark) DarkColors else LightColors
    MaterialTheme(colorScheme = colors, content = content)
}
