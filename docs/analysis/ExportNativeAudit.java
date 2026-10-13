// Export selected decompiler results to a private investigation directory.
// This does not patch executable bytes or supply code to the new application.
// @category Analysis.IDMAudit

import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.charset.StandardCharsets;
import java.io.BufferedWriter;

public class ExportNativeAudit extends GhidraScript {
    private String quote(String text) {
        return "\"" + text.replace("\\", "\\\\").replace("\"", "\\\"")
            .replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t") + "\"";
    }

    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) throw new IllegalArgumentException("OUTPUT_DIRECTORY NAME_FILTER_CSV_OR_ALL");
        Path directory = Path.of(args[0]);
        Files.createDirectories(directory);
        String[] filters = args[1].split(",");
        DecompInterface decompiler = new DecompInterface();
        if (!decompiler.openProgram(currentProgram)) throw new IllegalStateException("Cannot open program for decompilation");
        int count = 0;
        try (BufferedWriter metadata = Files.newBufferedWriter(directory.resolve("functions.jsonl"), StandardCharsets.UTF_8)) {
            FunctionIterator functions = currentProgram.getFunctionManager().getFunctions(true);
            while (functions.hasNext() && !monitor.isCancelled()) {
                Function function = functions.next();
                if (function.isExternal()) continue;
                boolean selected = args[1].equals("ALL");
                for (String filter : filters) if (function.getName().contains(filter)) selected = true;
                if (!selected) continue;
                DecompileResults result = decompiler.decompileFunction(function, 40, monitor);
                String file = function.getEntryPoint().toString() + ".c";
                if (result.decompileCompleted()) Files.writeString(directory.resolve(file), result.getDecompiledFunction().getC(), StandardCharsets.UTF_8);
                StringBuilder called = new StringBuilder("[");
                for (Function target : function.getCalledFunctions(monitor)) {
                    if (called.length() > 1) called.append(",");
                    called.append(quote(target.getName()+"@"+target.getEntryPoint()));
                }
                called.append("]");
                metadata.write("{\"program\":"+quote(currentProgram.getName())+
                    ",\"name\":"+quote(function.getName())+
                    ",\"address\":"+quote(function.getEntryPoint().toString())+
                    ",\"signature\":"+quote(function.getSignature().toString())+
                    ",\"body_bytes\":"+function.getBody().getNumAddresses()+
                    ",\"completed\":"+result.decompileCompleted()+
                    ",\"error\":"+quote(result.getErrorMessage())+
                    ",\"pseudocode_file\":"+quote(file)+",\"calls\":"+called+"}\n");
                count++;
            }
        } finally {
            decompiler.dispose();
        }
        println("IDM_AUDIT_EXPORTED_FUNCTIONS=" + count);
    }
}
