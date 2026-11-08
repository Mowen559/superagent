import java.nio.file.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Base64;
import org.mozilla.javascript.*;

// Execute only the outer action in a bounded safe scope. Never execute eval input.
class CaptureMobileEval {
    public static void main(String[] args) throws Exception {
        final long deadline=System.nanoTime()+5_000_000_000L;
        ContextFactory factory=new ContextFactory(){
            protected void observeInstructionCount(Context cx,int count){
                if(System.nanoTime()>deadline)throw new Error("diagnostic instruction deadline");
            }
        };
        Context c=factory.enterContext();c.setLanguageVersion(200);c.setOptimizationLevel(-1);
        c.setInstructionObserverThreshold(10000);c.setClassShutter(name->false);
        try {
            ScriptableObject scope=c.initSafeStandardObjects();
            ScriptableObject.putProperty(scope,"eval",new BaseFunction(){
                public Object call(Context cx,Scriptable s,Scriptable self,Object[] values){
                    String source=Context.toString(values[0]);String message="";int line=0;boolean pass=true;
                    try{cx.compileString(source,"captured_eval",1,null);}
                    catch(RhinoException error){pass=false;line=error.lineNumber();message=error.details();}
                    try {
                        byte[] digest=MessageDigest.getInstance("SHA-256").digest(source.getBytes(StandardCharsets.UTF_8));
                        StringBuilder hash=new StringBuilder();for(byte b:digest)hash.append(String.format("%02x",b&255));
                        System.out.println("EVAL\t"+pass+"\t"+line+"\t"+source.length()+"\t"+hash+"\t"+Base64.getEncoder().encodeToString(message.getBytes(StandardCharsets.UTF_8)));
                    }catch(Exception error){throw new RuntimeException(error);}
                    throw ScriptRuntime.typeError("diagnostic capture stops execution");
                }
            });
            ScriptableObject.putProperty(scope,"base64Decode",new BaseFunction(){
                public Object call(Context cx,Scriptable s,Scriptable self,Object[] values){
                    return new String(Base64.getDecoder().decode(Context.toString(values[0])),StandardCharsets.UTF_8);
                }
            });
            try{c.evaluateString(scope,Files.readString(Path.of(args[0])),"outer_action",1,null);}
            catch(RhinoException error){System.out.println("STOP\t"+Base64.getEncoder().encodeToString(error.details().getBytes(StandardCharsets.UTF_8)));}
        }finally{Context.exit();}
    }
}
