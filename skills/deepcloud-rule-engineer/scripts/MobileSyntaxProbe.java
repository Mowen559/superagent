import java.nio.file.*;
import org.mozilla.javascript.*;

// Compile only: never execute rule code, request a URL, or open an Android UI.
class MobileSyntaxProbe {
    public static void main(String[] paths) throws Exception {
        Context context = Context.enter();
        context.setLanguageVersion(200);
        context.setOptimizationLevel(-1);
        boolean failed = false;
        try {
            for (String file : paths) {
                try {
                    context.compileString(Files.readString(Path.of(file)), "rule", 1, null);
                    System.out.println("PASS\t0\t");
                } catch (RhinoException error) {
                    failed = true;
                    System.out.println("FAIL\t" + error.lineNumber() + "\t" + error.details().replace('\n', ' ').replace('\t', ' '));
                }
            }
        } finally { Context.exit(); }
        if (failed) System.exit(1);
    }
}
