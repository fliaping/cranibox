package com.cranibox.android.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp

/** Lightweight Markdown: headings, lists, fenced code, bold/italic, inline code. */
@Composable
fun MarkdownText(text: String, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    val typography = MaterialTheme.typography
    SelectionContainer(modifier) {
        Column {
            parseBlocks(text).forEach { block ->
                when (block) {
                    is MdBlock.Code -> Text(
                        text = block.body.trimEnd(),
                        style = typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 6.dp)
                            .background(scheme.surfaceContainerHighest, MaterialTheme.shapes.small)
                            .horizontalScroll(rememberScrollState())
                            .padding(12.dp),
                    )
                    is MdBlock.Heading -> Text(
                        text = block.text,
                        style = when (block.level) {
                            1 -> typography.titleLarge
                            2 -> typography.titleMedium
                            else -> typography.titleSmall
                        },
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
                    )
                    is MdBlock.Paragraph -> Text(
                        text = inline(block.text),
                        style = typography.bodyLarge,
                        modifier = Modifier.padding(vertical = 2.dp),
                    )
                }
            }
        }
    }
}

private sealed class MdBlock {
    data class Heading(val level: Int, val text: String) : MdBlock()
    data class Paragraph(val text: String) : MdBlock()
    data class Code(val body: String) : MdBlock()
}

private fun parseBlocks(text: String): List<MdBlock> {
    val blocks = mutableListOf<MdBlock>()
    val lines = text.replace("\r\n", "\n").split('\n')
    var i = 0
    val paragraph = StringBuilder()
    fun flush() {
        val body = paragraph.toString().trimEnd()
        if (body.isNotBlank()) blocks += MdBlock.Paragraph(body)
        paragraph.clear()
    }
    while (i < lines.size) {
        val line = lines[i]
        when {
            line.startsWith("```") -> {
                flush()
                val body = StringBuilder()
                i++
                while (i < lines.size && !lines[i].startsWith("```")) {
                    body.appendLine(lines[i]); i++
                }
                blocks += MdBlock.Code(body.toString())
            }
            line.startsWith("### ") -> { flush(); blocks += MdBlock.Heading(3, line.drop(4)) }
            line.startsWith("## ") -> { flush(); blocks += MdBlock.Heading(2, line.drop(3)) }
            line.startsWith("# ") -> { flush(); blocks += MdBlock.Heading(1, line.drop(2)) }
            line.matches(Regex("""^\s*[-*]\s+.*""")) -> {
                flush()
                val items = StringBuilder()
                while (i < lines.size && lines[i].matches(Regex("""^\s*[-*]\s+.*"""))) {
                    items.append("• ").append(lines[i].trim().dropWhile { it == '-' || it == '*' || it == ' ' }).append('\n')
                    i++
                }
                i--
                blocks += MdBlock.Paragraph(items.toString().trimEnd())
            }
            line.isBlank() -> flush()
            else -> {
                if (paragraph.isNotEmpty()) paragraph.append('\n')
                paragraph.append(line)
            }
        }
        i++
    }
    flush()
    return blocks
}

@Composable
private fun inline(text: String) = buildAnnotatedString {
    val regex = Regex("""(\*\*[^*]+?\*\*|\*[^*]+?\*|`[^`]+?`)""")
    var last = 0
    regex.findAll(text).forEach { match ->
        append(text.substring(last, match.range.first))
        val token = match.value
        when {
            token.startsWith("**") -> withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(token.removeSurrounding("**")) }
            token.startsWith("*") -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(token.removeSurrounding("*")) }
            else -> withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = MaterialTheme.colorScheme.surfaceContainerHigh)) {
                append(token.removeSurrounding("`"))
            }
        }
        last = match.range.last + 1
    }
    append(text.substring(last))
}
