use super::escape::{expand_percent_b, shell_quote_form};
use super::float::format_float;
use super::number::{invalid_number_error, parse_f64, parse_i64, parse_u64};
use super::FormatSpec;

/// One queued diagnostic: the bare builtin_error body plus whether it is
/// fatal (GNU conversion_error -> exit status 1). The tescape missing-digit
/// messages are builtin_error without conversion_error, so they stay
/// non-fatal; every numeric parse/ERANGE diagnostic is fatal.
pub(super) type Diagnostic = (String, bool);

pub(super) fn format_value(
    value: &str,
    spec: &FormatSpec,
    arg_present: bool,
) -> (String, bool, Vec<Diagnostic>) {
    let mut stop_output = false;
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut invalid_number: Option<String> = None;
    // GNU printf.def units model (printf_builtin + printstr/printwidestr):
    //   * `%ls`/`%S`/`%lc`/`%C` (longform or the uppercase conversion) go
    //     through printwidestr and count CHARACTERS, but only when the
    //     locale is multibyte (locale_mb_cur_max > 1, printf.def:496-547);
    //   * plain `%s`/`%c` are handed to the C library printf via the PF
    //     macro / take arg[0] in getchr, so width and precision count
    //     BYTES in every locale;
    //   * `%b`/`%q`/`%Q` go through printstr over the byte string, so
    //     their precision is a BYTE count as well.
    let wide = spec.wide || matches!(spec.specifier, 'S' | 'C');
    let byte_mode = !(wide && crate::locale::is_multi_byte());
    // GNU printf.def:681-694: a %Q precision that overflows decodes to -1
    // (no truncation of the unquoted argument); the effective precision is
    // then re-derived from the quoted length in printstr.
    let effective_precision = if spec.specifier == 'Q' && spec.inline_precision_overflow {
        None
    } else {
        spec.precision
    };
    let rendered = match spec.specifier {
        's' | 'S' => truncate_precision_locale(value.to_string(), effective_precision, byte_mode),
        'b' => {
            let (expanded, stop, diagnostic) = expand_percent_b(value);
            stop_output = stop;
            // GNU tescape's missing-digit diagnostic is builtin_error-only
            // (no conversion_error): non-fatal, prefixed by render_one_pass.
            if let Some(body) = diagnostic {
                diagnostics.push((body, false));
            }
            truncate_precision_locale(expanded, effective_precision, byte_mode)
        }
        'q' => truncate_precision_locale(
            shell_quote_form(value, spec.alternate_form),
            effective_precision,
            byte_mode,
        ),
        'Q' => shell_quote_form(
            &truncate_precision_locale(value.to_string(), effective_precision, byte_mode),
            spec.alternate_form,
        ),
        'c' | 'C' => first_char_or_byte(value, byte_mode),
        'd' | 'i' => {
            let parsed = parse_i64(value);
            invalid_number = parsed.invalid;
            format_signed_integer(parsed.value, spec)
        }
        'u' => {
            let parsed = parse_u64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value, 10, false, spec)
        }
        'x' => {
            let parsed = parse_u64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value, 16, false, spec)
        }
        'X' => {
            let parsed = parse_u64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value, 16, true, spec)
        }
        'o' => {
            let parsed = parse_u64(value);
            invalid_number = parsed.invalid;
            format_unsigned_integer(parsed.value, 8, false, spec)
        }
        'f' | 'F' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, 'f')
        }
        'e' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, 'e')
        }
        'E' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, 'E')
        }
        'g' | 'G' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, spec.specifier)
        }
        'a' | 'A' => {
            let parsed = parse_f64(value);
            invalid_number = parsed.invalid;
            format_float(parsed.value, spec, spec.specifier)
        }
        other => {
            let mut fallback = String::from('%');
            fallback.push(other);
            fallback
        }
    };

    let mut width_spec = spec.clone();
    if spec.precision.is_some() && matches!(spec.specifier, 'd' | 'i' | 'u' | 'x' | 'X' | 'o') {
        width_spec.zero_pad = false;
    }
    // GNU getintmax/getuintmax (printf.def:1416+): a PRESENT but empty
    // numeric operand makes strtoimax leave ep==s, so chk_converror calls
    // sh_invalidnum (`printf: : invalid number') and renders 0; a MISSING
    // argument silently converts as 0. Checked after the match arms because
    // parse_i64("") itself reports no issue.
    let numeric_specifier = matches!(
        spec.specifier,
        'd' | 'i' | 'u' | 'x' | 'X' | 'o' | 'f' | 'F' | 'e' | 'E' | 'g' | 'G' | 'a' | 'A'
    );
    if numeric_specifier && arg_present && value.is_empty() && invalid_number.is_none() {
        invalid_number = Some(String::new());
    }
    if let Some(value) = invalid_number {
        diagnostics.push((invalid_number_error(&value), true));
    }
    (
        apply_width_locale(rendered, &width_spec, byte_mode),
        stop_output,
        diagnostics,
    )
}

pub(super) fn truncate_precision(value: String, precision: Option<usize>) -> String {
    let Some(precision) = precision else {
        return value;
    };
    value.chars().take(precision).collect()
}

/// Locale-aware precision: character count in a multibyte locale, raw byte
/// count otherwise. Byte truncation is not rewound to a character boundary,
/// which is why a cut can leave a dangling multibyte lead byte behind.
fn truncate_precision_locale(value: String, precision: Option<usize>, byte_mode: bool) -> String {
    let Some(precision) = precision else {
        return value;
    };
    if !byte_mode {
        return value.chars().take(precision).collect();
    }
    let raw = locale_byte_span(&value);
    if raw.len() <= precision {
        return value;
    }
    crate::executor::substitution_metadata::bytes_to_shell_text(&raw[..precision])
}

/// `%c`/`%lc`: GNU getchr returns arg[0] -- the first raw BYTE for the
/// narrow form in every locale; the wide form (%lc/%C, multibyte locale)
/// converts the first CHARACTER (getwidechar/mbrtowc). Both an empty and a
/// missing argument yield a NUL byte (getchr's `return ('\0')`, PF prints
/// it), so `%c` with no argument emits one NUL (printf.tests `printf
/// '%c\n'` case).
fn first_char_or_byte(value: &str, byte_mode: bool) -> String {
    if !byte_mode {
        return value.chars().next().unwrap_or('\0').to_string();
    }
    let raw = locale_byte_span(value);
    if raw.is_empty() {
        return crate::executor::substitution_metadata::bytes_to_shell_text(&[0]);
    }
    crate::executor::substitution_metadata::bytes_to_shell_text(&raw[..1])
}

/// Locale-aware width: counts bytes in a single-byte locale so the padding
/// reaches the byte target, not the character target.
fn apply_width_locale(value: String, spec: &FormatSpec, byte_mode: bool) -> String {
    let Some(width) = spec.width else {
        return value;
    };

    let len = if byte_mode {
        locale_byte_span(&value).len()
    } else {
        value.chars().count()
    };
    if len >= width {
        return value;
    }

    let pad = width - len;
    let pad_char = if spec.zero_pad && !spec.left_adjust {
        '0'
    } else {
        ' '
    };
    let padding: String = std::iter::repeat(pad_char).take(pad).collect();

    if spec.left_adjust {
        format!("{value}{padding}")
    } else if spec.zero_pad && matches!(value.chars().next(), Some('+' | '-' | ' ')) {
        let mut chars = value.chars();
        let sign = chars.next().unwrap_or_default();
        let rest: String = chars.collect();
        format!("{sign}{padding}{rest}")
    } else {
        format!("{padding}{value}")
    }
}

/// The byte view of a word: raw-byte marker pairs (substitution_metadata)
/// decode back to the bytes they carry, everything else keeps its UTF-8 bytes.
fn locale_byte_span(value: &str) -> Vec<u8> {
    let sentinel = char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
        .expect("raw-byte sentinel is a valid char");
    if value.contains(sentinel) {
        crate::executor::substitution_metadata::decode_raw_byte_markers(value.as_bytes())
    } else {
        value.as_bytes().to_vec()
    }
}

fn format_unsigned_integer(value: u64, radix: u32, uppercase: bool, spec: &FormatSpec) -> String {
    let mut rendered = match (radix, uppercase) {
        (10, _) => value.to_string(),
        (8, _) => format!("{value:o}"),
        (16, false) => format!("{value:x}"),
        (16, true) => format!("{value:X}"),
        _ => value.to_string(),
    };

    rendered = apply_integer_precision(rendered, value == 0, spec.precision);

    if !spec.alternate_form {
        return rendered;
    }

    match (radix, uppercase) {
        (8, _) if !rendered.starts_with('0') => format!("0{rendered}"),
        (16, false) if value != 0 => format!("0x{rendered}"),
        (16, true) if value != 0 => format!("0X{rendered}"),
        _ => rendered,
    }
}

fn format_signed_integer(value: i64, spec: &FormatSpec) -> String {
    let mut rendered =
        apply_integer_precision(value.unsigned_abs().to_string(), value == 0, spec.precision);
    if value < 0 {
        rendered.insert(0, '-');
    } else if spec.explicit_sign {
        rendered.insert(0, '+');
    } else if spec.leading_space_sign {
        rendered.insert(0, ' ');
    }
    rendered
}

fn apply_integer_precision(mut digits: String, is_zero: bool, precision: Option<usize>) -> String {
    let Some(precision) = precision else {
        return digits;
    };
    if precision == 0 && is_zero {
        return String::new();
    }
    let len = digits.chars().count();
    if len < precision {
        let padding: String = std::iter::repeat('0').take(precision - len).collect();
        digits = format!("{padding}{digits}");
    }
    digits
}

pub(super) fn apply_width(value: String, spec: &FormatSpec) -> String {
    let Some(width) = spec.width else {
        return value;
    };

    let len = value.chars().count();
    if len >= width {
        return value;
    }

    let pad = width - len;
    let pad_char = if spec.zero_pad && !spec.left_adjust {
        '0'
    } else {
        ' '
    };
    let padding: String = std::iter::repeat(pad_char).take(pad).collect();

    if spec.left_adjust {
        format!("{value}{padding}")
    } else if spec.zero_pad && matches!(value.chars().next(), Some('+' | '-' | ' ')) {
        let mut chars = value.chars();
        let sign = chars.next().unwrap_or_default();
        let rest: String = chars.collect();
        format!("{sign}{padding}{rest}")
    } else {
        format!("{padding}{value}")
    }
}
