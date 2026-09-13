using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Automation;
using System.Windows.Forms;

public static class KlyneDesktop {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct MouseInput { public int dx,dy; public uint data,flags,time; public UIntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct KeyInput { public ushort key,scan; public uint flags,time; public UIntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct InputUnion { [FieldOffset(0)] public MouseInput mouse; [FieldOffset(0)] public KeyInput key; }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint type; public InputUnion data; }
    public static bool InputStarted=false;
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window,uint command);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr window);
    delegate bool EnumProc(IntPtr window,IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr param);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr window,StringBuilder title,int count);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr window,uint flags);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr window,int show);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] static extern uint SendInput(uint count,Input[] inputs,int size);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    static KlyneDesktop() { SetProcessDPIAware(); }
    public static void CheckEmergency() {
        Point cursor; GetCursorPos(out cursor);
        if((GetAsyncKeyState(0x13)&0x8000)!=0 || (cursor.X>=0 && cursor.X<=2 && cursor.Y>=0 && cursor.Y<=2))
            throw new InvalidOperationException("Emergency stop: Pause key or pointer at the primary screen's top-left corner.");
    }
    // User-input takeover detection (audit Phase 6): the desktop is shared
    // with the user, so every input primitive snapshots the pointer before
    // acting and verifies it afterwards. The agent knows exactly where it
    // put the pointer (Click/Scroll/Drag set it explicitly; keyboard input
    // never moves it), so any displacement means the user — or something
    // else — grabbed the mouse mid-operation and the effect is uncertain.
    static Point CursorNow() { Point p; GetCursorPos(out p); return p; }
    static void ExpectCursor(Point want,string op) {
        var after=CursorNow();
        if(after.X!=want.X || after.Y!=want.Y)
            throw new Exception(op+" did not land as sent: the pointer moved during input. The user may have taken over; outcome uncertain.");
    }
    // Password-focus refusal across primitives (audit Phase 6): the
    // ElementAction path already rejects password controls, but raw
    // Type/Press act on whatever holds focus. Refuse there too.
    static void RefusePasswordFocus() {
        try {
            var focused=AutomationElement.FocusedElement;
            if(focused!=null && focused.Current.IsPassword)
                throw new Exception("Password fields require your input.");
        } catch(Exception e) { if(e.Message.Contains("Password"))throw; }
    }
    static IntPtr Window(string id,bool foreground) {
        CheckEmergency(); long raw;
        if(!long.TryParse(id,out raw) || raw==0) throw new Exception("Invalid window handle. Observe first.");
        var window=new IntPtr(raw);
        if(!IsWindowVisible(window)) throw new Exception("Window is no longer visible. Observe again.");
        if(foreground && (GetForegroundWindow()!=window || IsIconic(window))) throw new Exception("The target is not foreground. Observe and focus it first.");
        return window;
    }
    static void Send(params Input[] inputs) {
        CheckEmergency();
        InputStarted=true;
        if(SendInput((uint)inputs.Length,inputs,Marshal.SizeOf(typeof(Input)))!=(uint)inputs.Length)
            throw new Exception("Windows blocked input. Elevated or secure windows may not be controllable.");
    }
    static Input Key(ushort key,ushort scan,uint flags) {return new Input{type=1,data=new InputUnion{key=new KeyInput{key=key,scan=scan,flags=flags}}};}
    static Input Mouse(uint flags,uint data) {return new Input{type=0,data=new InputUnion{mouse=new MouseInput{flags=flags,data=data}}};}
    public static void Focus(string id) {
        var window=Window(id,false);InputStarted=true;if(IsIconic(window))ShowWindow(window,9);
        SetForegroundWindow(window);Thread.Sleep(180);
        if(GetForegroundWindow()!=window)throw new Exception("Windows refused foreground focus. Select the target app and retry.");
    }
    public static void CheckLayout(string id,int left,int top,int width,int height) {
        var window=Window(id,true);Rect r;GetWindowRect(window,out r);
        if(r.Left!=left || r.Top!=top || r.Right-r.Left!=width || r.Bottom-r.Top!=height)
            throw new Exception("Window layout changed. Observe again before using coordinates.");
        if(!IsWindowEnabled(window))throw new Exception("Window is blocked by a modal dialog. Observe its owned windows.");
    }
    public static void Click(string id,int x,int y,string button) {
        var window=Window(id,true);var point=new Point{X=x,Y=y};
        if(GetAncestor(WindowFromPoint(point),2)!=window)throw new Exception("Click target is covered by another window or outside the target. Observe again.");
        if(!SetCursorPos(x,y))throw new Exception("Could not move pointer.");
        if(button=="right")Send(Mouse(8,0),Mouse(16,0));
        else if(button=="left")Send(Mouse(2,0),Mouse(4,0));
        else if(button=="double") {Send(Mouse(2,0),Mouse(4,0));Thread.Sleep(70);Send(Mouse(2,0),Mouse(4,0));}
        else throw new Exception("Button must be left, right or double.");
        ExpectCursor(point,"Click");
    }
    public static void Drag(string id,int x1,int y1,int x2,int y2) {
        var window=Window(id,true);
        var start=new Point{X=x1,Y=y1};var end=new Point{X=x2,Y=y2};
        if(GetAncestor(WindowFromPoint(start),2)!=window)throw new Exception("Drag start is covered by another window or outside the target. Observe again.");
        if(GetAncestor(WindowFromPoint(end),2)!=window)throw new Exception("Drag end is covered by another window or outside the target. Observe again.");
        if(!SetCursorPos(x1,y1))throw new Exception("Could not move pointer.");
        Send(Mouse(2,0));
        for(int step=1;step<=12;step++) {
            CheckEmergency();
            int x=x1+(x2-x1)*step/12,y=y1+(y2-y1)*step/12;
            if(!SetCursorPos(x,y)) { Send(Mouse(4,0)); throw new Exception("Could not move pointer during drag."); }
            Thread.Sleep(8);
        }
        Send(Mouse(4,0));
        ExpectCursor(end,"Drag");
    }
    public static void Type(string id,string text) {
        if(text.Length>4000)throw new Exception("Type at most 4000 characters per action.");
        RefusePasswordFocus();var before=CursorNow();
        foreach(char c in text) {Window(id,true);Send(Key(0,c,4),Key(0,c,6));}
        ExpectCursor(before,"Type");
    }
    public static void Press(string id,string chord) {
        Window(id,true);RefusePasswordFocus();var before=CursorNow();var inputs=new List<Input>();var modifiers=new List<ushort>();var parts=chord.ToUpperInvariant().Split('+');
        for(int i=0;i<parts.Length-1;i++) {
            ushort modifier=parts[i]=="CTRL"?(ushort)17:parts[i]=="ALT"?(ushort)18:parts[i]=="SHIFT"?(ushort)16:(ushort)0;
            if(modifier==0)throw new Exception("Unknown key modifier.");modifiers.Add(modifier);inputs.Add(Key(modifier,0,0));
        }
        var keyName=parts[parts.Length-1];ushort code=0;
        var keys=new Dictionary<string,ushort>{{"ENTER",13},{"TAB",9},{"ESC",27},{"ESCAPE",27},{"BACKSPACE",8},{"DELETE",46},{"HOME",36},{"END",35},{"LEFT",37},{"UP",38},{"RIGHT",39},{"DOWN",40},{"PAGEUP",33},{"PAGEDOWN",34},{"SPACE",32},{"F4",115},{"F5",116},{"F6",117}};
        if(!keys.TryGetValue(keyName,out code) && keyName.Length==1 && char.IsLetterOrDigit(keyName[0]))code=keyName[0];
        if(code==0)throw new Exception("Unsupported key.");
        inputs.Add(Key(code,0,0));inputs.Add(Key(code,0,2));
        for(int i=modifiers.Count-1;i>=0;i--)inputs.Add(Key(modifiers[i],0,2));Send(inputs.ToArray());
        ExpectCursor(before,"Press");
    }
    public static void Scroll(string id,int ticks) {
        var window=Window(id,true);Rect r;GetWindowRect(window,out r);var p=new Point{X=(r.Left+r.Right)/2,Y=(r.Top+r.Bottom)/2};
        if(GetAncestor(WindowFromPoint(p),2)!=window)throw new Exception("Scroll target is obscured.");
        if(ticks==0 || Math.Abs(ticks)>10)throw new Exception("Scroll ticks must be between -10 and 10, excluding zero.");
        SetCursorPos(p.X,p.Y);Send(Mouse(0x0800,unchecked((uint)(ticks*120))));
        ExpectCursor(p,"Scroll");
    }
    static IEnumerable<AutomationElement> Elements(IntPtr window) {
        var queue=new Queue<AutomationElement>();queue.Enqueue(AutomationElement.FromHandle(window));int count=0;
        while(queue.Count>0 && count++<250) {
            var item=queue.Dequeue();yield return item;
            AutomationElement child=null;try{child=TreeWalker.ControlViewWalker.GetFirstChild(item);}catch{}
            int children=0;while(child!=null && children++<100 && queue.Count<250) {queue.Enqueue(child);try{child=TreeWalker.ControlViewWalker.GetNextSibling(child);}catch{break;}}
        }
    }
    static string Id(AutomationElement element) {return string.Join(".",element.GetRuntimeId());}
    public static void ElementAction(string windowId,string id,string text,bool fill) {
        var window=Window(windowId,true);var before=CursorNow();
        foreach(var element in Elements(window)) {
            try {if(Id(element)!=id)continue;}catch{continue;}
            if(element.Current.IsPassword)throw new Exception("Password fields require your input.");
            if(!element.Current.IsEnabled || element.Current.IsOffscreen)throw new Exception("Control is not available.");
            object pattern;
            if(fill) {
                if(text.Length>4000)throw new Exception("Fill at most 4000 characters.");
                if(!element.TryGetCurrentPattern(ValuePattern.Pattern,out pattern))throw new Exception("Control has no Value pattern; click and type instead.");
                InputStarted=true;((ValuePattern)pattern).SetValue(text);
            } else {
                if(element.TryGetCurrentPattern(InvokePattern.Pattern,out pattern)){InputStarted=true;((InvokePattern)pattern).Invoke();}
                else if(element.TryGetCurrentPattern(SelectionItemPattern.Pattern,out pattern)){InputStarted=true;((SelectionItemPattern)pattern).Select();}
                else if(element.TryGetCurrentPattern(ExpandCollapsePattern.Pattern,out pattern)){InputStarted=true;((ExpandCollapsePattern)pattern).Expand();}
                else throw new Exception("Control has no supported action pattern. Observe and use keyboard navigation or visible coordinates.");
            }
            ExpectCursor(before,"Element input");
            return;
        }
        throw new Exception("Control is stale or unavailable. Observe again.");
    }
    // Clipboard access runs on STA threads: the PowerShell host is MTA and
    // System.Windows.Forms.Clipboard refuses to work there. Content is
    // capped like typed input; reads are sensitive user data — the caller
    // treats them as untrusted and never logs them beyond task evidence.
    public static void ClipboardSet(string text) {
        CheckEmergency();
        if(text==null)text="";
        if(text.Length>4000)throw new Exception("Clipboard holds at most 4000 characters per action.");
        Exception failure=null;
        var worker=new Thread(()=>{try{Clipboard.SetText(text);}catch(Exception e){failure=e;}});
        worker.SetApartmentState(ApartmentState.STA);worker.Start();
        if(!worker.Join(5000))throw new Exception("Clipboard write timed out; outcome uncertain.");
        if(failure!=null)throw new Exception("Clipboard write failed: "+failure.Message);
        InputStarted=true;
    }
    public static string ClipboardGet() {
        CheckEmergency();
        string result="";Exception failure=null;
        var worker=new Thread(()=>{try{if(Clipboard.ContainsText())result=Clipboard.GetText();}catch(Exception e){failure=e;}});
        worker.SetApartmentState(ApartmentState.STA);worker.Start();
        if(!worker.Join(5000))throw new Exception("Clipboard read timed out; outcome uncertain.");
        if(failure!=null)throw new Exception("Clipboard read failed: "+failure.Message);
        if(result.Length>4000)result=result.Substring(0,4000)+"[truncated]";
        return result;
    }
    public static object Observe(string imagePath) {        CheckEmergency();var screen=SystemInformation.VirtualScreen;
        if(screen.Width<=0 || screen.Height<=0 || screen.Width>20000 || screen.Height>12000)throw new Exception("No supported interactive desktop.");
        var width=Math.Min(1600,screen.Width);var height=Math.Max(1,(int)((long)screen.Height*width/screen.Width));
        using(var bitmap=new Bitmap(screen.Width,screen.Height)) {
            using(var graphics=Graphics.FromImage(bitmap))graphics.CopyFromScreen(screen.Left,screen.Top,0,0,screen.Size,CopyPixelOperation.SourceCopy);
            using(var resized=new Bitmap(bitmap,width,height))resized.Save(imagePath,ImageFormat.Png);
        }
        var foreground=GetForegroundWindow();var windows=new List<object>();
        EnumWindows(delegate(IntPtr handle,IntPtr param){
            if(windows.Count>=60 || !IsWindowVisible(handle))return true;
            var title=new StringBuilder(400);GetWindowText(handle,title,title.Capacity);if(title.Length==0)return true;
            uint process;GetWindowThreadProcessId(handle,out process);string process_started=null,process_name=null;
            try {using(var p=System.Diagnostics.Process.GetProcessById((int)process)){process_started=p.StartTime.ToUniversalTime().ToFileTimeUtc().ToString();process_name=p.ProcessName;}}catch{}
            Rect r;GetWindowRect(handle,out r);windows.Add(new {window=handle.ToInt64().ToString(),title=title.ToString(),process_id=process,process_started=process_started,process_name=process_name,minimized=IsIconic(handle),enabled=IsWindowEnabled(handle),owner=GetWindow(handle,4).ToInt64().ToString(),left=r.Left,top=r.Top,width=r.Right-r.Left,height=r.Bottom-r.Top});return true;
        },IntPtr.Zero);
        var controls=new List<object>();string treeError=null;
        try { foreach(var element in Elements(foreground)) {
            if(controls.Count>=120)break;
            try {
                var c=element.Current;if(c.IsOffscreen || c.IsPassword)continue;var r=c.BoundingRectangle;
                if(r.IsEmpty || double.IsInfinity(r.X))continue;string value=null;object pattern;
                if(element.TryGetCurrentPattern(ValuePattern.Pattern,out pattern))value=((ValuePattern)pattern).Current.Value;
                else if(element.TryGetCurrentPattern(TextPattern.Pattern,out pattern))value=((TextPattern)pattern).DocumentRange.GetText(1201);
                bool value_truncated=value!=null && value.Length>1200;
                if(value_truncated)value=value.Substring(0,1200);
                var name=c.Name;if(name.Length>200)name=name.Substring(0,200);
                controls.Add(new {element=Id(element),name=name,type=c.ControlType.ProgrammaticName,enabled=c.IsEnabled,patterns=Array.ConvertAll(element.GetSupportedPatterns(),p=>p.ProgrammaticName),value=value,value_truncated=value_truncated,x=(int)((r.X-screen.Left)*width/screen.Width),y=(int)((r.Y-screen.Top)*height/screen.Height),width=(int)(r.Width*width/screen.Width),height=(int)(r.Height*height/screen.Height)});
            }catch{}
        }}catch(Exception e){treeError=e.Message;}
        return new {foreground=foreground.ToInt64().ToString(),screen=new {left=screen.Left,top=screen.Top,width=screen.Width,height=screen.Height,image_width=width,image_height=height},windows=windows,controls=controls,tree_error=treeError};
    }
}
