package io.github.enrell.lain.player

import android.content.Context
import android.opengl.GLES20
import android.opengl.GLES30
import androidx.media3.common.VideoFrameProcessingException
import androidx.media3.common.util.Size
import androidx.media3.common.util.UnstableApi
import androidx.media3.effect.BaseGlShaderProgram
import androidx.media3.effect.GlEffect
import androidx.media3.effect.GlShaderProgram
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/*
 * Anime4K in the player: the web client's engine
 * (web/src/lib/player/webgl/engine.ts), on OpenGL ES 3 inside Media3's
 * frame processor. The shader packs are the same mpv hook files, parsed
 * the same way; each hook is a pass into a half-float texture and the
 * last one draws the frame Media3 presents.
 */

/** The presets, with the web client's ids. */
enum class VideoEffect(val id: String, val label: String) {
    Off("off", "Off"),
    ModeA("anime4k-a", "Anime4K Mode A"),
    ModeAA("anime4k-aa", "Anime4K Mode A+A"),
    Lite("anime4k-lite", "Anime4K Lite"),
    DogX2("anime4k-dog-x2", "Anime4K DoG ×2");

    companion object {
        fun of(id: String): VideoEffect = entries.firstOrNull { it.id == id } ?: Off
    }
}

/** One mpv hook: a fragment pass, what it reads, where it saves, at what size, and when. */
class HookPass(
    val description: String,
    val binds: List<String>,
    val save: String,
    val width: String?,
    val height: String?,
    val whenExpr: String?,
    val source: String,
)

private val DIRECTIVE = Regex("""^//!(\w+)\s+(.*)$""")

/** Splits an mpv shader file into its hooks (`//!DESC` starts one). */
fun parseHooks(source: String): List<HookPass> {
    val sections = mutableListOf<MutableList<String>>()
    for (line in source.lines()) {
        if (line.startsWith("//!DESC ")) sections += mutableListOf<String>()
        sections.lastOrNull()?.add(line)
    }
    return sections.map { lines ->
        val directives = HashMap<String, MutableList<String>>()
        val code = StringBuilder()
        for (line in lines) {
            val m = DIRECTIVE.find(line)
            if (m != null) directives.getOrPut(m.groupValues[1]) { mutableListOf() }.add(m.groupValues[2].trim()) else code.append(line).append('\n')
        }
        HookPass(
            description = directives["DESC"]?.firstOrNull() ?: "Anime4K pass",
            binds = directives["BIND"].orEmpty(),
            save = directives["SAVE"]?.firstOrNull() ?: "MAIN",
            width = directives["WIDTH"]?.firstOrNull(),
            height = directives["HEIGHT"]?.firstOrNull(),
            whenExpr = directives["WHEN"]?.firstOrNull(),
            source = code.toString(),
        )
    }
}

private val REFERENCED = Regex("""\b([A-Za-z][A-Za-z0-9_]*)_(?:texOff|tex|pos|pt|size)\b""")

/** Every texture a hook reads: what it binds and what its code names. */
fun inputsOf(hook: HookPass): List<String> =
    (hook.binds + REFERENCED.findAll(hook.source).map { it.groupValues[1] }.filter { it != "L" }).distinct()

/** mpv's reverse-Polish size and condition expressions (`OUTPUT.w MAIN.w / 1.2 >`). */
fun evaluate(source: String, dimensions: Map<String, Pair<Int, Int>>): Double {
    val stack = ArrayDeque<Double>()
    for (token in source.trim().split(Regex("\\s+"))) {
        token.toDoubleOrNull()?.let { stack.addLast(it) } ?: run {
            val field = Regex("""^([A-Za-z][A-Za-z0-9_]*)\.([wh])$""").find(token)
            if (field != null) {
                val size = dimensions[field.groupValues[1]] ?: error("Anime4K dimension $token is unavailable")
                stack.addLast((if (field.groupValues[2] == "w") size.first else size.second).toDouble())
            } else {
                val right = stack.removeLastOrNull() ?: error("Invalid Anime4K expression: $source")
                val left = stack.removeLastOrNull() ?: error("Invalid Anime4K expression: $source")
                stack.addLast(
                    when (token) {
                        "+" -> left + right
                        "-" -> left - right
                        "*" -> left * right
                        "/" -> left / right
                        ">" -> if (left > right) 1.0 else 0.0
                        "<" -> if (left < right) 1.0 else 0.0
                        ">=" -> if (left >= right) 1.0 else 0.0
                        "<=" -> if (left <= right) 1.0 else 0.0
                        else -> error("Unknown Anime4K expression operator $token")
                    },
                )
            }
        }
    }
    check(stack.size == 1 && stack.first().isFinite()) { "Invalid Anime4K expression: $source" }
    return stack.first()
}

// The web client's DoG x2 pack, as fragment hooks.
private val DOG_X2 = listOf(
    HookPass(
        "Anime4K DoG luminance", listOf("MAIN"), "LINELUMA", null, null, null,
        "vec4 hook() { return vec4(dot(MAIN_tex(MAIN_pos), vec4(0.299, 0.587, 0.114, 0.0)), 0.0, 0.0, 1.0); }",
    ),
    HookPass(
        "Anime4K DoG Gaussian X", listOf("LINELUMA"), "MMKERNEL", null, null, null,
        """vec4 hook() {
  vec2 d = vec2(LINELUMA_pt.x, 0.0);
  float c = LINELUMA_tex(LINELUMA_pos).r;
  float l = LINELUMA_tex(LINELUMA_pos - d).r;
  float r = LINELUMA_tex(LINELUMA_pos + d).r;
  float g = (LINELUMA_tex(LINELUMA_pos - d * 2.0).r + LINELUMA_tex(LINELUMA_pos + d * 2.0).r) * 0.06136;
  g += (l + r) * 0.24477 + c * 0.38774;
  return vec4(g, min(min(l, c), r), max(max(l, c), r), 1.0);
}""",
    ),
    HookPass(
        "Anime4K DoG Gaussian Y", listOf("MMKERNEL"), "LINEKERNEL", null, null, null,
        """vec4 hook() {
  vec2 d = vec2(0.0, MMKERNEL_pt.y);
  vec3 c = MMKERNEL_tex(MMKERNEL_pos).rgb;
  vec3 a = MMKERNEL_tex(MMKERNEL_pos - d).rgb;
  vec3 b = MMKERNEL_tex(MMKERNEL_pos + d).rgb;
  float g = (MMKERNEL_tex(MMKERNEL_pos - d * 2.0).r + MMKERNEL_tex(MMKERNEL_pos + d * 2.0).r) * 0.06136;
  g += (a.r + b.r) * 0.24477 + c.r * 0.38774;
  return vec4(g, min(min(a.g, c.g), b.g), max(max(a.b, c.b), b.b), 1.0);
}""",
    ),
    HookPass(
        "Anime4K DoG apply x2", listOf("MAIN", "LINELUMA", "LINEKERNEL"), "MAIN", "NATIVE.w 2 *", "NATIVE.h 2 *", null,
        """vec4 hook() {
  float lum = LINELUMA_tex(LINELUMA_pos).r;
  vec3 gauss = LINEKERNEL_tex(LINEKERNEL_pos).rgb;
  float correction = clamp((lum - gauss.r) * 0.8 + lum, gauss.g, gauss.b) - lum;
  return MAIN_tex(MAIN_pos) + vec4(correction, correction, correction, 0.0);
}""",
    ),
)

/** The shader files of a preset, in order (the web client's chains). */
fun packFiles(effect: VideoEffect): List<String> = when (effect) {
    VideoEffect.ModeA -> listOf("Clamp_Highlights", "Restore_CNN_M", "Upscale_CNN_x2_M", "AutoDownscalePre_x2", "AutoDownscalePre_x4", "Upscale_CNN_x2_S")
    VideoEffect.ModeAA -> listOf("Clamp_Highlights", "Restore_CNN_M", "Upscale_CNN_x2_M", "Restore_CNN_S", "AutoDownscalePre_x2", "AutoDownscalePre_x4", "Upscale_CNN_x2_S")
    VideoEffect.Lite -> listOf("Clamp_Highlights", "Restore_CNN_S", "Deblur_DoG", "Darken_VeryFast")
    else -> emptyList()
}

/** The hooks of a preset, read from the packaged shader files. */
fun hooksOf(context: Context, effect: VideoEffect): List<HookPass> =
    if (effect == VideoEffect.DogX2) DOG_X2
    else packFiles(effect).flatMap { name -> parseHooks(context.assets.open("Anime4K_$name.glsl").bufferedReader().use { it.readText() }) }

private const val VERTEX = """#version 300 es
precision highp float;
out vec2 v_uv;
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  v_uv = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}"""

private const val COPY = """#version 300 es
precision highp float;
uniform sampler2D u_image;
in vec2 v_uv;
out vec4 out_color;
void main() { out_color = texture(u_image, v_uv); }"""

/** A hook as a GLSL ES 3 fragment shader: mpv's `NAME_tex`, `NAME_pos`, `NAME_pt`, `NAME_size`, `NAME_texOff`. */
fun fragmentSource(hook: HookPass, inputs: List<String>): String {
    // Convolution layers run at mediump (twice the rate on most phone GPUs,
    // within 1/255 of highp); coordinates and sizes stay highp.
    val p = if (Regex("""\bmat4\(""").containsMatchIn(hook.source)) "mediump" else "highp"
    val helpers = inputs.joinToString("") { name ->
        """
uniform highp sampler2D u_$name;
uniform highp vec2 size_$name;
#define ${name}_pos v_uv
#define ${name}_pt (1.0 / size_$name)
#define ${name}_size size_$name
$p vec4 ${name}_tex(highp vec2 uv) { return texture(u_$name, uv); }
$p vec4 ${name}_texOff(highp vec2 offset) { return texture(u_$name, v_uv + offset / size_$name); }
"""
    }
    return "#version 300 es\nprecision $p float;\nprecision highp sampler2D;\nin highp vec2 v_uv;\nout $p vec4 out_color;\n$helpers\n${hook.source}\nvoid main() { out_color = hook(); }\n"
}

/**
 * The effect to hand to `ExoPlayer.setVideoEffects`. [display] is the
 * size the picture is shown at, in pixels: the packs upscale only as far
 * as it needs (their `WHEN` conditions). A failure calls [onError] once
 * and the frames pass through untouched from then on: an effect that
 * cannot run must not stop the video.
 */
@UnstableApi
class Anime4kEffect(
    private val effect: VideoEffect,
    private val display: () -> Pair<Int, Int>,
    private val onError: (String) -> Unit,
) : GlEffect {
    override fun toGlShaderProgram(context: Context, useHdr: Boolean): GlShaderProgram =
        Anime4kProgram(context.applicationContext, effect, display, onError)
}

private class Texture(val id: Int, val width: Int, val height: Int)

private class Prepared(val hook: HookPass, val program: Int, val inputs: List<String>, val samplers: IntArray, val sizes: IntArray)

/** [output] is null for the pass that draws the presented frame. */
private class Pass(val prepared: Prepared, val bindings: List<Texture?>, val output: Texture?, val width: Int, val height: Int)

@UnstableApi
private class Anime4kProgram(
    private val context: Context,
    private val effect: VideoEffect,
    private val display: () -> Pair<Int, Int>,
    private val onError: (String) -> Unit,
) : BaseGlShaderProgram(/* useHighPrecisionColorComponents = */ false, /* texturePoolCapacity = */ 1) {
    private var vertex = 0
    private var copy = 0
    private var copySampler = 0
    private var frameBuffer = 0
    private var vertexArray = 0
    private var prepared: List<Prepared>? = null
    private val textures = mutableListOf<Texture>()
    private var passes: List<Pass> = emptyList()
    private var configured = ""
    private var outputSize = Size(1, 1)
    private var broken = false

    private fun fail(reason: String) {
        if (broken) return
        broken = true
        passes = emptyList()
        onError(reason)
    }

    private fun shader(type: Int, source: String): Int {
        val id = GLES20.glCreateShader(type)
        GLES20.glShaderSource(id, source)
        GLES20.glCompileShader(id)
        val ok = IntArray(1)
        GLES20.glGetShaderiv(id, GLES20.GL_COMPILE_STATUS, ok, 0)
        if (ok[0] == 0) {
            val log = GLES20.glGetShaderInfoLog(id)
            GLES20.glDeleteShader(id)
            error(log.ifBlank { "GLSL compile error" })
        }
        return id
    }

    private fun program(fragment: String): Int {
        val frag = shader(GLES20.GL_FRAGMENT_SHADER, fragment)
        val id = GLES20.glCreateProgram()
        GLES20.glAttachShader(id, vertex)
        GLES20.glAttachShader(id, frag)
        GLES20.glLinkProgram(id)
        GLES20.glDeleteShader(frag)
        val ok = IntArray(1)
        GLES20.glGetProgramiv(id, GLES20.GL_LINK_STATUS, ok, 0)
        if (ok[0] == 0) {
            val log = GLES20.glGetProgramInfoLog(id)
            GLES20.glDeleteProgram(id)
            error(log.ifBlank { "GLSL link error" })
        }
        return id
    }

    /** Compiles the preset's hooks once, on the GL thread. */
    private fun compile() {
        if (vertex != 0) return
        val extensions = GLES20.glGetString(GLES20.GL_EXTENSIONS).orEmpty()
        check("EXT_color_buffer_half_float" in extensions || "EXT_color_buffer_float" in extensions) {
            "this GPU cannot render to floating-point textures"
        }
        vertex = shader(GLES20.GL_VERTEX_SHADER, VERTEX)
        copy = program(COPY)
        copySampler = GLES20.glGetUniformLocation(copy, "u_image")
        val ids = IntArray(1)
        GLES20.glGenFramebuffers(1, ids, 0)
        frameBuffer = ids[0]
        GLES30.glGenVertexArrays(1, ids, 0)
        vertexArray = ids[0]
        val units = IntArray(1)
        GLES20.glGetIntegerv(GLES20.GL_MAX_TEXTURE_IMAGE_UNITS, units, 0)
        prepared = hooksOf(context, effect).map { hook ->
            val inputs = inputsOf(hook)
            check(inputs.size <= units[0]) { "${hook.description}: too many textures" }
            val id = program(fragmentSource(hook, inputs))
            Prepared(
                hook, id, inputs,
                IntArray(inputs.size) { GLES20.glGetUniformLocation(id, "u_${inputs[it]}") },
                IntArray(inputs.size) { GLES20.glGetUniformLocation(id, "size_${inputs[it]}") },
            )
        }
    }

    private fun texture(width: Int, height: Int): Texture {
        val ids = IntArray(1)
        GLES20.glGenTextures(1, ids, 0)
        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, ids[0])
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)
        GLES30.glTexStorage2D(GLES20.GL_TEXTURE_2D, 1, GLES30.GL_RGBA16F, width, height)
        return Texture(ids[0], width, height).also { textures += it }
    }

    /** Lays the passes out for this frame size and the size it is shown at. */
    private fun prepare(width: Int, height: Int) {
        for (t in textures) GLES20.glDeleteTextures(1, intArrayOf(t.id), 0)
        textures.clear()
        val (viewW, viewH) = display()
        val fit = min(max(1, viewW).toFloat() / width, max(1, viewH).toFloat() / height)
        val target = max(1, (width * fit).roundToInt()) to max(1, (height * fit).roundToInt())
        val limit = IntArray(1)
        GLES20.glGetIntegerv(GLES20.GL_MAX_TEXTURE_SIZE, limit, 0)
        val dimensions = hashMapOf("NATIVE" to (width to height), "MAIN" to (width to height), "OUTPUT" to target)
        // A null binding is the frame Media3 hands in, bound at draw time.
        val bindings = HashMap<String, Texture?>().apply { put("NATIVE", null); put("MAIN", null) }
        val out = mutableListOf<Pass>()
        GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, frameBuffer)
        for (p in prepared.orEmpty()) {
            val hook = p.hook
            dimensions["HOOKED"] = dimensions.getValue("MAIN")
            bindings["HOOKED"] = bindings["MAIN"]
            if (hook.whenExpr != null && evaluate(hook.whenExpr, dimensions) == 0.0) continue
            val main = dimensions.getValue("MAIN")
            val w = max(1, (hook.width?.let { evaluate(it, dimensions) } ?: main.first.toDouble()).roundToInt())
            val h = max(1, (hook.height?.let { evaluate(it, dimensions) } ?: main.second.toDouble()).roundToInt())
            check(w <= limit[0] && h <= limit[0]) { "Anime4K texture ${w}x$h exceeds GPU limits" }
            val inputs = p.inputs.map { name ->
                check(bindings.containsKey(name)) { "${hook.description}: missing $name" }
                bindings[name]
            }
            val output = texture(w, h)
            GLES20.glFramebufferTexture2D(GLES20.GL_FRAMEBUFFER, GLES20.GL_COLOR_ATTACHMENT0, GLES20.GL_TEXTURE_2D, output.id, 0)
            check(GLES20.glCheckFramebufferStatus(GLES20.GL_FRAMEBUFFER) == GLES20.GL_FRAMEBUFFER_COMPLETE) { "${hook.description}: incomplete render target" }
            out += Pass(p, inputs, output, w, h)
            bindings[hook.save] = output
            dimensions[hook.save] = w to h
        }
        // The last pass that runs draws the presented frame, so it needs no texture of its own.
        val last = out.lastOrNull()
        if (last?.output != null) {
            GLES20.glDeleteTextures(1, intArrayOf(last.output.id), 0)
            textures.remove(last.output)
            out[out.lastIndex] = Pass(last.prepared, last.bindings, null, last.width, last.height)
        }
        passes = out
        outputSize = if (last != null) Size(last.width, last.height) else Size(width, height)
        android.util.Log.i("lain", "anime4k ${effect.id}: ${out.size} passes, ${width}x$height -> ${outputSize.width}x${outputSize.height} (shown at ${target.first}x${target.second})")
    }

    override fun configure(inputWidth: Int, inputHeight: Int): Size {
        if (broken) return Size(inputWidth, inputHeight)
        val (viewW, viewH) = display()
        val key = "$inputWidth:$inputHeight:$viewW:$viewH"
        if (key != configured) {
            try {
                compile()
                prepare(inputWidth, inputHeight)
                configured = key
            } catch (e: Throwable) {
                fail(e.message ?: e.toString())
                return Size(inputWidth, inputHeight)
            }
        }
        return outputSize
    }

    @Throws(VideoFrameProcessingException::class)
    override fun drawFrame(inputTexId: Int, presentationTimeUs: Long) {
        // Media3 bound the frame to present; the intermediate passes use ours.
        val presented = IntArray(1)
        GLES20.glGetIntegerv(GLES20.GL_FRAMEBUFFER_BINDING, presented, 0)
        val viewport = IntArray(4)
        GLES20.glGetIntegerv(GLES20.GL_VIEWPORT, viewport, 0)
        GLES30.glBindVertexArray(vertexArray)
        try {
            for (pass in passes) {
                if (pass.output != null) {
                    GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, frameBuffer)
                    GLES20.glFramebufferTexture2D(GLES20.GL_FRAMEBUFFER, GLES20.GL_COLOR_ATTACHMENT0, GLES20.GL_TEXTURE_2D, pass.output.id, 0)
                    GLES20.glViewport(0, 0, pass.width, pass.height)
                } else {
                    GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, presented[0])
                    GLES20.glViewport(viewport[0], viewport[1], viewport[2], viewport[3])
                }
                GLES20.glUseProgram(pass.prepared.program)
                pass.bindings.forEachIndexed { i, binding ->
                    GLES20.glActiveTexture(GLES20.GL_TEXTURE0 + i)
                    GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, binding?.id ?: inputTexId)
                    GLES20.glUniform1i(pass.prepared.samplers[i], i)
                    // The source frame is the size the passes were laid out for.
                    val w = binding?.width ?: sourceWidth
                    val h = binding?.height ?: sourceHeight
                    GLES20.glUniform2f(pass.prepared.sizes[i], w.toFloat(), h.toFloat())
                }
                GLES20.glDrawArrays(GLES20.GL_TRIANGLES, 0, 3)
            }
            if (passes.isEmpty()) {
                // Off, broken, or every hook skipped: the frame as it came.
                if (copy == 0) {
                    vertex = if (vertex == 0) shader(GLES20.GL_VERTEX_SHADER, VERTEX) else vertex
                    copy = program(COPY)
                    copySampler = GLES20.glGetUniformLocation(copy, "u_image")
                }
                GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, presented[0])
                GLES20.glViewport(viewport[0], viewport[1], viewport[2], viewport[3])
                GLES20.glUseProgram(copy)
                GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
                GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, inputTexId)
                GLES20.glUniform1i(copySampler, 0)
                GLES20.glDrawArrays(GLES20.GL_TRIANGLES, 0, 3)
            }
            // getError syncs the pipeline: prove the first frames, then sample.
            frames = (frames + 1) % 240
            if ((frames < 3 || frames == 0) && passes.isNotEmpty() && GLES20.glGetError() != GLES20.GL_NO_ERROR) {
                fail("the GPU rejected an Anime4K frame")
            }
        } catch (e: Throwable) {
            fail(e.message ?: e.toString())
        } finally {
            GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
            GLES30.glBindVertexArray(0)
            GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, presented[0])
        }
    }

    private var frames = 0
    private val sourceWidth: Int get() = configured.split(':').getOrNull(0)?.toIntOrNull() ?: 1
    private val sourceHeight: Int get() = configured.split(':').getOrNull(1)?.toIntOrNull() ?: 1

    override fun release() {
        super.release()
        for (t in textures) GLES20.glDeleteTextures(1, intArrayOf(t.id), 0)
        textures.clear()
        prepared?.forEach { GLES20.glDeleteProgram(it.program) }
        if (copy != 0) GLES20.glDeleteProgram(copy)
        if (vertex != 0) GLES20.glDeleteShader(vertex)
        if (frameBuffer != 0) GLES20.glDeleteFramebuffers(1, intArrayOf(frameBuffer), 0)
        if (vertexArray != 0) GLES30.glDeleteVertexArrays(1, intArrayOf(vertexArray), 0)
    }
}
