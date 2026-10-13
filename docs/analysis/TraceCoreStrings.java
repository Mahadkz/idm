// Locate native functions referencing selected HTTP/recovery strings.
// Decompiled output belongs only in a private investigation directory.
// @category Analysis.IDMAudit
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.Reference;
import java.nio.file.*;
import java.nio.charset.StandardCharsets;
import java.io.BufferedWriter;
import java.util.*;

public class TraceCoreStrings extends GhidraScript {
    private String quote(String s) {
        return "\""+s.replace("\\","\\\\").replace("\"","\\\"")
            .replace("\n","\\n").replace("\r","\\r").replace("\t","\\t")+"\"";
    }
    public void run() throws Exception {
        Path output=Path.of(getScriptArgs()[0]);
        Files.createDirectories(output);
        String[] terms={"content-range","accept-ranges","if-range","range:","dwnldata",".log",".bin","resume"};
        Set<Function> selected=new LinkedHashSet<>();
        try(BufferedWriter refs=Files.newBufferedWriter(output.resolve("string_references.jsonl"),StandardCharsets.UTF_8)) {
            DataIterator data=currentProgram.getListing().getDefinedData(true);
            while(data.hasNext()&&!monitor.isCancelled()) {
                Data value=data.next();
                Object object=value.getValue();
                if(!(object instanceof String)) continue;
                String text=(String)object;
                String lower=text.toLowerCase(Locale.ROOT);
                boolean match=false;
                for(String term:terms) if(lower.contains(term)) match=true;
                if(!match) continue;
                List<String> callers=new ArrayList<>();
                for(Reference ref:getReferencesTo(value.getAddress())) {
                    Function fn=getFunctionContaining(ref.getFromAddress());
                    if(fn==null) continue;
                    selected.add(fn);
                    callers.add(quote(fn.getEntryPoint().toString()));
                }
                refs.write("{\"address\":"+quote(value.getAddress().toString())+",\"text\":"+quote(text)+",\"referencing_functions\":["+String.join(",",callers)+"]}\n");
            }
        }
        DecompInterface decompiler=new DecompInterface();
        if(!decompiler.openProgram(currentProgram)) throw new IllegalStateException("Cannot open program");
        try(BufferedWriter metadata=Files.newBufferedWriter(output.resolve("functions.jsonl"),StandardCharsets.UTF_8)) {
            for(Function fn:selected) {
                if(monitor.isCancelled()) break;
                DecompileResults result=decompiler.decompileFunction(fn,30,monitor);
                if(result.decompileCompleted()) Files.writeString(output.resolve(fn.getEntryPoint()+".c"),result.getDecompiledFunction().getC(),StandardCharsets.UTF_8);
                List<String> calls=new ArrayList<>();
                for(Function target:fn.getCalledFunctions(monitor)) calls.add(quote(target.getName()+"@"+target.getEntryPoint()));
                metadata.write("{\"name\":"+quote(fn.getName())+",\"address\":"+quote(fn.getEntryPoint().toString())+",\"completed\":"+result.decompileCompleted()+",\"error\":"+quote(result.getErrorMessage())+",\"calls\":["+String.join(",",calls)+"]}\n");
            }
        } finally {decompiler.dispose();}
        println("IDM_CORE_STRING_REFERENCING_FUNCTIONS="+selected.size());
    }
}
