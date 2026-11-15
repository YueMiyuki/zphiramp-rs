/**
 * Escaping used by every golden-vector file: one record per line, fields separated by TAB.
 * Inside a field, backslash, TAB, CR, LF and every other char below U+0020 (plus U+007F,
 * U+2028, U+2029 and unpaired surrogates) are written as backslash escapes; the Rust side
 * reverses this in tests/support/mod.rs (an unpaired surrogate becomes U+FFFD there).
 */
final class Esc {
    private Esc() {}

    static String esc(String s) {
        StringBuilder b = new StringBuilder(s.length() + 8);
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '\\' -> b.append("\\\\");
                case '\t' -> b.append("\\t");
                case '\n' -> b.append("\\n");
                case '\r' -> b.append("\\r");
                default -> {
                    boolean pairedHigh = Character.isHighSurrogate(c) && i + 1 < s.length()
                            && Character.isLowSurrogate(s.charAt(i + 1));
                    boolean pairedLow = Character.isLowSurrogate(c) && i > 0
                            && Character.isHighSurrogate(s.charAt(i - 1));
                    boolean lone = Character.isSurrogate(c) && !pairedHigh && !pairedLow;
                    if (c < 0x20 || c == 0x7f || c == 0x2028 || c == 0x2029 || lone) {
                        b.append(String.format("\\u%04x", (int) c));
                    } else {
                        b.append(c);
                    }
                }
            }
        }
        return b.toString();
    }
}
