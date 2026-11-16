// Writes fdlibm 5.3's own results, from the JDK's StrictMath (a strict, unfused Java port of fdlibm 5.3):
//   java crates/slidock-math/tests/data/Jdk.java [data directory, default crates/slidock-math/tests/data]
// Run it after generate.mjs: it reads the inputs of fdlibm.tsv and the sweep rows of fdlibm-sweep.tsv.
// - fdlibm-jdk.tsv:        `fn  x  [y]  result` for every input of fdlibm.tsv;
// - fdlibm-sweep-jdk.tsv:  `fn  dist  params  count  sha256`, the sweeps of fdlibm-sweep.tsv plus cbrt and hypot
//   rows and the branch rows of BRANCHES, PAIRS and KERNELS, drawn as samples.mjs draws them.
// StrictMath.hypot lacks FreeBSD's fix for a subnormal smaller argument, so sweeps leave out those pairs
// (`subnormalHypot`), as the Rust test does.

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;

public class Jdk {
    static Path DIR = Path.of("crates/slidock-math/tests/data");

    static double call(String fn, double[] a) {
        return switch (fn) {
            case "sin" -> StrictMath.sin(a[0]);
            case "cos" -> StrictMath.cos(a[0]);
            case "tan" -> StrictMath.tan(a[0]);
            case "asin" -> StrictMath.asin(a[0]);
            case "acos" -> StrictMath.acos(a[0]);
            case "atan" -> StrictMath.atan(a[0]);
            case "exp" -> StrictMath.exp(a[0]);
            case "log" -> StrictMath.log(a[0]);
            case "cbrt" -> StrictMath.cbrt(a[0]);
            case "atan2" -> StrictMath.atan2(a[0], a[1]);
            case "hypot" -> StrictMath.hypot(a[0], a[1]);
            default -> throw new IllegalArgumentException(fn);
        };
    }

    static long canonicalBits(double x) {
        return Double.isNaN(x) ? 0x7ff8000000000000L : Double.doubleToRawLongBits(x);
    }

    static String hex(long bits) {
        return String.format("%016x", bits);
    }

    /** Whether hypot(x, y) takes fdlibm's subnormal-b branch, where FreeBSD's fix changes the result. */
    static boolean subnormalHypot(double x, double y) {
        int ha = (int) (Double.doubleToRawLongBits(x) >>> 32) & 0x7fffffff;
        int hb = (int) (Double.doubleToRawLongBits(y) >>> 32) & 0x7fffffff;
        double b = y;
        if (hb > ha) {
            int t = ha;
            ha = hb;
            hb = t;
            b = x;
        }
        return ha - hb <= 0x3c00000 && hb <= 0x000fffff && b != 0;
    }

    /** splitmix64, as samples.mjs. */
    static final class Rng {
        long state;

        Rng(long seed) {
            state = seed;
        }

        long next() {
            state += 0x9e3779b97f4a7c15L;
            long z = state;
            z = (z ^ (z >>> 30)) * 0xbf58476d1ce4e5b9L;
            z = (z ^ (z >>> 27)) * 0x94d049bb133111ebL;
            return z ^ (z >>> 31);
        }

        double unit() {
            return (double) (next() >>> 11) / 9007199254740992.0;
        }

        int below(double n) {
            return (int) Math.floor(unit() * n);
        }
    }

    static double pow2(int e) {
        return e >= -1022 ? Double.longBitsToDouble((long) (e + 1023) << 52) : Double.longBitsToDouble(1L << (e + 1074));
    }

    static long seedFor(String key) {
        long h = 0xcbf29ce484222325L;
        for (char c : key.toCharArray()) h = (h ^ c) * 0x100000001b3L;
        return h;
    }

    /**
     * fdlibm's branch thresholds on high words: per function, the constants it compares a high word with (the
     * port's comparisons, which tests/jdk.rs checks are all here) and log's mantissa thresholds. Each gets a
     * `highWord` row at the constant, one below and one above, for each sign listed.
     */
    static final String[][] BRANCHES = {
        {"sin", "+-", "3e400000 3fe921fb 3ff921fb 4002d97c 413921fb 7ff00000"},
        {"cos", "+-", "3e400000 3fd33333 3fe90000 3fe921fb 3ff921fb 4002d97c 413921fb 7ff00000"},
        {"tan", "+-", "3e300000 3fe59428 3fe921fb 3ff921fb 4002d97c 413921fb 7ff00000"},
        {"asin", "+-", "3e400000 3fe00000 3fef3333 3ff00000"},
        {"acos", "+-", "3c600000 3fe00000 3ff00000"},
        {"atan", "+-", "3e200000 3fdc0000 3fe60000 3ff30000 40038000 44100000 7ff00000"},
        {"exp", "+-", "3e300000 3fd62e42 3ff0a2b2 40862e42 7ff00000"},
        // |f| < 2^-20 at mantissas ffffe..0 and the reduction's 6147a, 6a09c and 6b851, at k = 0 and k ≠ 0.
        {"log", "+", "00100000 7ff00000 3feffffe 3ff00000 3ff6147a 3ff6a09c 3ff6b851 400ffffe 40100000 4006147a 4006a09c 4006b851"},
        {"cbrt", "+-", "00100000 7ff00000"},
    };

    /**
     * Two-argument branch rows, `y x` for atan2 and `x y` for hypot: atan2's |y/x| > 2^60 and |y|/x < -2^60 cuts
     * (high words 61·2^20 apart, and 60·2^20 the other way) in every quadrant and its infinities; hypot's 2^60 ratio
     * cut both ways round, its 2^500 and 2^-500 scalings, its subnormal cut and its infinities and NaNs (with b within
     * 2^60 of them, which the ratio cut would return first).
     */
    static final String[][] PAIRS = {
        {"atan2", "43c00000 3ff00000"}, {"atan2", "c3c00000 3ff00000"},
        {"atan2", "43c00000 bff00000"}, {"atan2", "c3c00000 bff00000"},
        {"atan2", "3c300000 3ff00000"}, {"atan2", "bc300000 3ff00000"},
        {"atan2", "3c300000 bff00000"}, {"atan2", "bc300000 bff00000"},
        {"atan2", "3ff00000 7ff00000"}, {"atan2", "3ff00000 fff00000"},
        {"atan2", "7ff00000 3ff00000"}, {"atan2", "fff00000 3ff00000"},
        {"hypot", "43b00000 3ff00000"}, {"hypot", "3ff00000 43b00000"},
        {"hypot", "5f300000 5f300000"}, {"hypot", "20b00000 20b00000"},
        {"hypot", "000fffff 000fffff"}, {"hypot", "7ff00000 7fe00000"},
    };

    /**
     * The trigonometric kernels' thresholds reached through the reduction, where the reduced argument has a tail:
     * fn, signs, constants. The offset after n·π/2 carries the high word.
     */
    static final String[][] KERNELS = {
        {"sin", "+-", "3e400000 3fd33333 3fe90000"},
        {"cos", "+-", "3e400000 3fd33333 3fe90000"},
        {"tan", "+-", "3e300000 3fe59428"},
    };

    static final int BRANCH_SAMPLES = 50000;
    static final int KERNEL_SAMPLES = 200000;

    /** `highWord` params: each constant c as the range c - 1, c, c + 1. */
    static String around(String... constants) {
        List<String> params = new ArrayList<>();
        for (String c : constants) params.add(String.format("%08x,3", (Long.parseLong(c, 16) - 1) & 0xffffffffL));
        return String.join(",", params);
    }

    static double highWord(double lo, double count, Rng rng) {
        long high = ((long) lo + rng.below(count)) & 0xffffffffL;
        long low = rng.next() & 0xffffffffL;
        return Double.longBitsToDouble((high << 32) | low);
    }

    static double draw(String dist, double[] p, int arg, Rng rng) {
        switch (dist) {
            case "uniform":
                return p[0] + (p[1] - p[0]) * rng.unit();
            case "signedPow2": {
                double u = rng.unit();
                return (u - 0.5) * pow2((int) p[0] + rng.below(p[1]));
            }
            case "positivePow2": {
                double u = rng.unit();
                return u * pow2((int) p[0] + rng.below(p[1]));
            }
            case "anyBits":
                return Double.longBitsToDouble(rng.next());
            case "subnormal":
                return Double.longBitsToDouble(rng.next() & 0x800fffffffffffffL);
            case "nearHalfPi": {
                int n = rng.below(p[0]);
                return Double.longBitsToDouble(Double.doubleToRawLongBits(n * (Math.PI / 2)) + rng.below(17) - 8);
            }
            case "nearOne": {
                double sign = rng.below(2) == 0 ? 1 : -1;
                return sign * (1 - pow2(-1 - rng.below(52)));
            }
            case "highWord": {
                int j = 2 * Math.min(arg, p.length / 2 - 1);
                return highWord(p[j], p[j + 1], rng);
            }
            case "reducedHighWord": {
                int n = rng.below(p[0]);
                return n * (Math.PI / 2) + highWord(p[1], p[2], rng);
            }
            default:
                throw new IllegalArgumentException(dist);
        }
    }

    public static void main(String[] args) throws Exception {
        if (args.length > 0) DIR = Path.of(args[0]);
        String engine = "# engine\tjava=" + System.getProperty("java.version") + "\tarch=" + System.getProperty("os.arch");
        List<String> vectors = new ArrayList<>(List.of(engine, "# fn\tx\t[y]\tresult"));
        for (String line : Files.readAllLines(DIR.resolve("fdlibm.tsv"))) {
            if (line.startsWith("#")) continue;
            String[] cols = line.split("\t");
            double[] a = new double[cols.length - 3];
            StringBuilder row = new StringBuilder(cols[0]);
            for (int i = 0; i < a.length; i++) {
                a[i] = Double.longBitsToDouble(Long.parseUnsignedLong(cols[2 + i], 16));
                row.append('\t').append(cols[2 + i]);
            }
            vectors.add(row.append('\t').append(hex(canonicalBits(call(cols[0], a)))).toString());
        }
        Files.write(DIR.resolve("fdlibm-jdk.tsv"), vectors, StandardCharsets.UTF_8);

        List<String[]> rows = new ArrayList<>();
        for (String line : Files.readAllLines(DIR.resolve("fdlibm-sweep.tsv"))) {
            if (!line.startsWith("#")) rows.add(line.split("\t", -1));
        }
        rows.add(new String[] {"cbrt", "signedPow2", "-1000,2000", "100000"});
        rows.add(new String[] {"cbrt", "anyBits", "", "50000"});
        rows.add(new String[] {"cbrt", "subnormal", "", "50000"});
        rows.add(new String[] {"hypot", "signedPow2", "-1000,2000", "100000"});
        rows.add(new String[] {"hypot", "signedPow2", "-20,40", "100000"});
        rows.add(new String[] {"hypot", "anyBits", "", "50000"});
        // n·π/2 for n < 32, where the medium reduction compares the high word with NPIO2_HW.
        for (String fn : List.of("sin", "cos", "tan")) rows.add(new String[] {fn, "nearHalfPi", "32", "50000"});
        String samples = Integer.toString(BRANCH_SAMPLES);
        for (String[] branch : BRANCHES) {
            for (String c : branch[2].split(" ")) {
                for (char sign : branch[1].toCharArray()) {
                    String word = sign == '+' ? c : Long.toHexString(Long.parseLong(c, 16) | 0x80000000L);
                    rows.add(new String[] {branch[0], "highWord", around(word), samples});
                }
            }
        }
        for (String[] pair : PAIRS) rows.add(new String[] {pair[0], "highWord", around(pair[1].split(" ")), samples});
        for (String[] kernel : KERNELS) {
            for (String c : kernel[2].split(" ")) {
                for (char sign : kernel[1].toCharArray()) {
                    String word = sign == '+' ? c : Long.toHexString(Long.parseLong(c, 16) | 0x80000000L);
                    rows.add(new String[] {kernel[0], "reducedHighWord", "10000," + around(word), Integer.toString(KERNEL_SAMPLES)});
                }
            }
        }
        List<String> sweeps = new ArrayList<>(List.of(engine, "# fn\tdist\tparams\tcount\tsha256"));
        for (String[] row : rows) {
            String fn = row[0], dist = row[1], params = row[2];
            int count = Integer.parseInt(row[3]);
            int radix = dist.equals("highWord") || dist.equals("reducedHighWord") ? 16 : 10;
            double[] p = params.isEmpty()
                    ? new double[0]
                    : java.util.Arrays.stream(params.split(",")).mapToDouble(v -> radix == 16 ? Long.parseLong(v, 16) : Double.parseDouble(v)).toArray();
            String key = fn + "\t" + dist + "\t" + params;
            Rng rng = new Rng(seedFor(key));
            int arity = fn.equals("atan2") || fn.equals("hypot") ? 2 : 1;
            MessageDigest sha = MessageDigest.getInstance("SHA-256");
            for (int i = 0; i < count; i++) {
                double[] a = new double[arity];
                for (int j = 0; j < arity; j++) a[j] = draw(dist, p, j, rng);
                if (fn.equals("hypot") && subnormalHypot(a[0], a[1])) continue;
                long bits = canonicalBits(call(fn, a));
                for (int k = 7; k >= 0; k--) sha.update((byte) (bits >>> (8 * k)));
            }
            sweeps.add(key + "\t" + count + "\t" + HexFormat.of().formatHex(sha.digest()));
        }
        Files.write(DIR.resolve("fdlibm-sweep-jdk.tsv"), sweeps, StandardCharsets.UTF_8);
    }
}
