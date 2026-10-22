package nl.merel.app

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** Merel's "Helder" colours, the desktop's theme tokens; every text colour meets WCAG 2.2 AA. */
@Immutable
data class Helder(
    val window: Color,
    val card: Color,
    val border: Color,
    val text: Color,
    val dim: Color,
    val accent: Color,
    val ink: Color,
    val onAccent: Color,
    val selected: Color,
    val flag: Color,
)

val LightHelder = Helder(
    window = Color(0xFFF3F4F6), card = Color(0xFFFFFFFF), border = Color(0xFFE3E5E8), text = Color(0xFF242424),
    dim = Color(0xFF616161), accent = Color(0xFF1F6FC9), ink = Color(0xFF1757A3), onAccent = Color(0xFFFFFFFF),
    selected = Color(0xFFDCE8F7), flag = Color(0xFFC4314B),
)

val DarkHelder = Helder(
    window = Color(0xFF1B1C1E), card = Color(0xFF26282B), border = Color(0xFF3A3D42), text = Color(0xFFEEEEEE),
    dim = Color(0xFFC8C8C8), accent = Color(0xFF6FA8EE), ink = Color(0xFF95C0F4), onAccent = Color(0xFF0B1A2E),
    selected = Color(0xFF1E3552), flag = Color(0xFFF07A8C),
)

val LocalHelder = staticCompositionLocalOf { LightHelder }

@Composable
fun MerelTheme(content: @Composable () -> Unit) {
    val h = if (isSystemInDarkTheme()) DarkHelder else LightHelder
    val base = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()
    val scheme = base.copy(
        primary = h.accent, onPrimary = h.onAccent, primaryContainer = h.selected, onPrimaryContainer = h.ink,
        secondaryContainer = h.selected, onSecondaryContainer = h.ink,
        background = h.window, onBackground = h.text, surface = h.window, onSurface = h.text,
        surfaceContainer = h.card, surfaceContainerLow = h.card, surfaceContainerHigh = h.card,
        onSurfaceVariant = h.dim, outline = h.border, outlineVariant = h.border, error = h.flag,
    )
    androidx.compose.runtime.CompositionLocalProvider(LocalHelder provides h) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}

/** helder::AVATAR_COLORS and its hash, so a person has the same colour as on the desktop. */
private val avatarColors = listOf(0xFF1F5FA8, 0xFF2E6E50, 0xFF6B4FA8, 0xFF8A5300, 0xFFA11F35, 0xFF0F666E, 0xFF4A4E54, 0xFF8C2F86)

fun avatarColor(address: String): Color {
    var hash = 0u
    for (b in address.lowercase().encodeToByteArray()) hash = hash * 31u + b.toUByte().toUInt()
    return Color(avatarColors[(hash % avatarColors.size.toUInt()).toInt()])
}

/** First letters of the first and last word, skipping what stands in brackets, as on the desktop. */
fun initials(name: String): String {
    val words = name.substringBefore('(').split(Regex("\\s+")).filter { it.firstOrNull()?.isLetterOrDigit() == true }
    return when (words.size) {
        0 -> ""
        1 -> words[0].take(1).uppercase()
        else -> words.first().take(1).uppercase() + words.last().take(1).uppercase()
    }
}
