//! Highlight Bar for Linux/X11. Requires an X11 compositor for translucency.
//! Run `highlight-bar-linux --help` for commands.
use std::{env, error::Error, fs, io::{self, BufRead}, path::PathBuf, sync::mpsc, thread, time::{Duration, Instant}};
use serde::{Deserialize, Serialize};
use x11rb::{connection::Connection, protocol::{xproto::*, shape::{self, SK, SO}, Event}, rust_connection::RustConnection, wrapper::ConnectionExt as _};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="camelCase")]
enum Mode { Off, BarOnly, BarAndSpotlight, ScreenTint }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="camelCase")]
enum Shape { Ruler, Line }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="camelCase")]
enum Orientation { Horizontal, Vertical }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="camelCase")]
enum Tracking { Mouse, Scroll, Keyboard }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all="camelCase")]
struct Settings {
    font_reference_size: f64,
    bar_opacity: f64,
    color_name: String,
    is_locked: bool,
    locked_anchor_x: Option<f64>,
    locked_anchor_y: Option<f64>,
    mode: Mode,
    spotlight_color_name: String,
    spotlight_opacity: f64,
    tint_color_name: String,
    tint_opacity: f64,
    bar_shape: Shape,
    bar_orientation: Orientation,
    tracking_source: Tracking,
    launch_at_login: bool,
    has_seen_onboarding: bool,
    per_app_enabled: bool,
    enabled_bundle_ids: Vec<String>,
    has_shown_hot_key_conflict_alert: bool,
}
impl Default for Settings {
    fn default() -> Self { Self {
        font_reference_size: 22.0, bar_opacity: 0.35, color_name: "Yellow".into(),
        is_locked: false, locked_anchor_x: None, locked_anchor_y: None,
        mode: Mode::BarOnly, spotlight_color_name: "Gray".into(), spotlight_opacity: 0.5,
        tint_color_name: "Yellow".into(), tint_opacity: 0.2,
        bar_shape: Shape::Ruler, bar_orientation: Orientation::Horizontal,
        tracking_source: Tracking::Mouse, launch_at_login: false, has_seen_onboarding: false,
        per_app_enabled: false, enabled_bundle_ids: vec![], has_shown_hot_key_conflict_alert: false,
    }}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Profile { name: String, settings: Settings, #[serde(rename="isBuiltIn")] is_built_in: bool }
fn builtins() -> Vec<Profile> {
    let mut dys = Settings::default(); dys.font_reference_size=26.0; dys.bar_opacity=0.30;
    let mut focus = Settings::default(); focus.font_reference_size=24.0; focus.bar_opacity=0.35;
    focus.mode=Mode::BarAndSpotlight; focus.spotlight_opacity=0.60;
    let mut vision = Settings::default(); vision.font_reference_size=48.0; vision.bar_opacity=0.55;
    vec![Profile{name:"Dyslexia".into(),settings:dys,is_built_in:true},
         Profile{name:"ADHD / Focus".into(),settings:focus,is_built_in:true},
         Profile{name:"Low vision".into(),settings:vision,is_built_in:true}]
}
fn config_dir() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".")).join("highlight-bar-linux")
}
fn load<T: serde::de::DeserializeOwned + Default>(file: &str) -> T {
    fs::read(config_dir().join(file)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
fn save<T: Serialize>(file: &str, data: &T) {
    let dir=config_dir(); if fs::create_dir_all(&dir).is_ok() {
        if let Ok(buf)=serde_json::to_vec_pretty(data) { let _=fs::write(dir.join(file),buf); }
    }
}
fn color(name: &str) -> [u8;3] {
    if name.len()==7 && name.starts_with('#') {
        if let Ok(n)=u32::from_str_radix(&name[1..],16) {
            return [(n>>16) as u8,(n>>8) as u8,n as u8];
        }
    }
    match name.to_ascii_lowercase().as_str() {
        "yellow"=>[255,204,0], "green"=>[52,199,89], "blue"=>[0,122,255],
        "pink"=>[255,45,85], "orange"=>[255,149,0], "gray"|"grey"=>[100,100,100],
        "black"=>[0,0,0], "white"=>[255,255,255], _=>[255,204,0],
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Rect { x: i32, y: i32, w: u32, h: u32 }
fn bar_rect(x: f64, y: f64, thickness: f64, orientation: Orientation, screen: Rect) -> Rect {
    let t=thickness.max(1.0);
    match orientation {
        Orientation::Horizontal => Rect { x:screen.x, y:(y-t/2.0).round().clamp(screen.y as f64,(screen.y+screen.h as i32) as f64-t) as i32,
            w:screen.w, h:t.min(screen.h as f64) as u32 },
        Orientation::Vertical => Rect { x:(x-t/2.0).round().clamp(screen.x as f64,(screen.x+screen.w as i32) as f64-t) as i32, y:screen.y,
            w:t.min(screen.w as f64) as u32, h:screen.h },
    }
}
fn thickness(s: &Settings) -> f64 {
    match s.bar_shape { Shape::Ruler => (s.font_reference_size*2.0).round().clamp(18.0,200.0),
        Shape::Line => (s.font_reference_size*0.3).round().clamp(2.0,40.0) }
}
fn intersect(a: Rect,b: Rect)->bool { a.x < b.x+b.w as i32 && a.x+a.w as i32>b.x && a.y<b.y+b.h as i32 && a.y+a.h as i32>b.y }
fn contains(r: Rect,x:f64,y:f64)->bool { x>=r.x as f64 && y>=r.y as f64 && x<(r.x+r.w as i32) as f64 && y<(r.y+r.h as i32) as f64 }

struct Window { id:WindowId, gc:Gcontext, frame:Rect, mapped:bool }
struct XApp { conn:RustConnection, root:WindowId, screen_no:usize, colormap:Colormap, visual:Visualid,
    bar:Window, overlay:Window, screens:Vec<Rect>, settings:Settings, profiles:Vec<Profile>,
    hidden:bool, tracked:(f64,f64), last_bar:Rect, last_root_size:(u32,u32),
    keys: Vec<(u8,u32)> }
impl XApp {
    fn create_window(conn:&RustConnection, root:WindowId, visual:Visualid, cmap:Colormap, frame:Rect)->Result<Window, Box<dyn Error>> {
        let id=conn.generate_id()?;
        conn.create_window(32,id,root,frame.x as i16,frame.y as i16,frame.w as u16,frame.h as u16,0,
            WindowClass::INPUT_OUTPUT,visual,&CreateWindowAux::new().colormap(cmap).override_redirect(1)
                .background_pixel(0).border_pixel(0).event_mask(EventMask::EXPOSURE))?;
        // An empty input region makes the visual window completely click-through.
        shape::rectangles(conn,SO::SET,SK::INPUT,ClipOrdering::UNSORTED,id,0,0,&[])?;
        let gc=conn.generate_id()?; conn.create_gc(gc,id,&CreateGCAux::new().graphics_exposures(0))?;
        Ok(Window{id,gc,frame,mapped:false})
    }
    fn new()->Result<Self,Box<dyn Error>> {
        let (conn,n)=x11rb::connect(None)?;
        let root_screen=&conn.setup().roots[n]; let root=root_screen.root;
        let visual=root_screen.allowed_depths.iter().find(|d| d.depth==32)
            .and_then(|d|d.visuals.iter().find(|v|v.class==VisualClass::TRUE_COLOR))
            .map(|v|v.visual_id).ok_or("No 32-bit ARGB X11 visual; enable a compositor")?;
        let size=(u32::from(root_screen.width_in_pixels),u32::from(root_screen.height_in_pixels));
        let cmap=conn.generate_id()?; conn.create_colormap(ColormapAlloc::NONE,cmap,root,visual)?;
        let whole=Rect{x:0,y:0,w:size.0,h:size.1};
        let bar=Self::create_window(&conn,root,visual,cmap,Rect{x:0,y:0,w:2,h:2})?;
        let overlay=Self::create_window(&conn,root,visual,cmap,whole)?;
        let screens=Self::monitors(&conn,root,whole);
        let settings:Settings=load("settings.json");
        let profiles:Vec<Profile>=load("profiles.json");
        let mut app=Self{conn,root,screen_no:n,colormap:cmap,visual,bar,overlay,screens,settings,
            profiles,hidden:false,tracked:(0.0,0.0),last_bar:Rect::default(),last_root_size:size,keys:vec![]};
        app.register_hotkeys()?;
        app.tracked=app.pointer()?;
        app.render(true)?;
        Ok(app)
    }
    fn monitors(conn:&RustConnection,root:WindowId,fallback:Rect)->Vec<Rect> {
        // XRandR monitors have root-window coordinates (top-left origin).
        use x11rb::protocol::randr;
        if let Ok(cookie)=randr::get_monitors(conn,root,true) {
            if let Ok(reply)=cookie.reply() {
                let v:Vec<Rect>=reply.monitors.iter().filter(|m|m.width>0&&m.height>0)
                    .map(|m|Rect{x:m.x.into(),y:m.y.into(),w:m.width.into(),h:m.height.into()}).collect();
                if !v.is_empty(){return v}
            }
        }
        vec![fallback]
    }
    fn pointer(&self)->Result<(f64,f64),Box<dyn Error>> {
        let p=self.conn.query_pointer(self.root)?.reply()?;
        Ok((f64::from(p.root_x),f64::from(p.root_y)))
    }
    fn hotkey_code(&self, keysym:u32)->Result<Option<u8>,Box<dyn Error>> {
        let setup=self.conn.setup(); let min=setup.min_keycode; let count=setup.max_keycode-min+1;
        let reply=self.conn.get_keyboard_mapping(min,count)?.reply()?;
        let each=reply.keysyms_per_keycode as usize;
        Ok(reply.keysyms.chunks(each).position(|row|row.contains(&keysym)).map(|i|min+i as u8))
    }
    fn register_one(&mut self, sym:u32, event:u32)->Result<(),Box<dyn Error>> {
        if let Some(keycode)=self.hotkey_code(sym)? {
            let mods=ModMask::M4|ModMask::SHIFT; // Super+Shift, Linux equivalent to Cmd+Shift.
            for extra in [ModMask::default(),ModMask::LOCK,ModMask::M2,ModMask::LOCK|ModMask::M2] {
                let _=self.conn.grab_key(false,self.root,mods|extra,keycode,GrabMode::ASYNC,GrabMode::ASYNC);
            }
            self.keys.push((keycode,event));
        } else { eprintln!("Missing key mapping for keysym {sym:x}"); }
        Ok(())
    }
    fn register_hotkeys(&mut self)->Result<(),Box<dyn Error>> {
        self.register_one(0x68,1)?; // h
        self.register_one(0x6c,2)?; // l
        self.register_one(0xff52,3)?; // Up
        self.register_one(0xff54,4)?; // Down
        self.conn.flush()?; Ok(())
    }
    fn screen_for(&self,x:f64,y:f64)->Rect {
        self.screens.iter().copied().find(|&r|contains(r,x,y))
            .unwrap_or_else(||self.screens[0])
    }
    fn anchor(&self)->(f64,f64) {
        if self.settings.is_locked {
            if let (Some(x),Some(y))=(self.settings.locked_anchor_x,self.settings.locked_anchor_y){return(x,y)}
        }
        self.tracked
    }
    fn effective_bar(&self)->Rect {
        let (x,y)=self.anchor(); bar_rect(x,y,thickness(&self.settings),self.settings.bar_orientation,self.screen_for(x,y))
    }
    fn set_frame(&self,w:&Window,frame:Rect)->Result<(),Box<dyn Error>> {
        self.conn.configure_window(w.id,&ConfigureWindowAux::new()
            .x(frame.x).y(frame.y).width(frame.w.max(1)).height(frame.h.max(1)).stack_mode(StackMode::ABOVE))?;
        Ok(())
    }
    fn rgba(bg:[u8;3],opacity:f64)->[u8;4] {
        let a=(opacity.clamp(0.0,1.0)*255.0).round() as u8;
        // X11 little-endian 32-bit ARGB: B,G,R,A; premultiplied for compositors.
        [((bg[2] as u16*a as u16)/255) as u8,
         ((bg[1] as u16*a as u16)/255) as u8,
         ((bg[0] as u16*a as u16)/255) as u8,a]
    }
    fn pixels(w:u32,h:u32,color:[u8;4],cut:Option<Rect>,origin:(i32,i32))->Vec<u8> {
        let mut buf=vec![0u8;(w*h*4) as usize];
        for yy in 0..h {for xx in 0..w {
            let gx=origin.0+xx as i32;let gy=origin.1+yy as i32;
            if cut.is_some_and(|c|contains(c,gx as f64,gy as f64)){continue}
            let pos=((yy*w+xx)*4) as usize; buf[pos..pos+4].copy_from_slice(&color);
        }}buf
    }
    fn upload(&self,w:&Window, pixels:&[u8],width:u32,height:u32)->Result<(),Box<dyn Error>> {
        // X11 normal request limit: split large images into row strips.
        let max_rows=(60000/width.max(1)).max(1);
        let mut y=0;
        while y<height {
            let rows=(height-y).min(max_rows);
            let start=(y*width*4) as usize; let end=((y+rows)*width*4) as usize;
            self.conn.put_image(ImageFormat::Z_PIXMAP,w.id,w.gc,width as u16,rows as u16,0,y as i16,0,32,&pixels[start..end])?;
            y+=rows;
        }
        Ok(())
    }
    fn render(&mut self, force:bool)->Result<(),Box<dyn Error>> {
        let s=&self.settings;
        let bar_visible=!self.hidden&&matches!(s.mode,Mode::BarOnly|Mode::BarAndSpotlight);
        let overlay_visible=!self.hidden&&matches!(s.mode,Mode::BarAndSpotlight|Mode::ScreenTint);
        let frame=self.effective_bar();
        if bar_visible {
            if force||frame!=self.last_bar {
                self.set_frame(&self.bar,frame)?;
                let c=color(&s.color_name); let fill=Self::rgba(c,s.bar_opacity);
                let mut pixels=Self::pixels(frame.w,frame.h,fill,None,(0,0));
                // 1 px outline, slightly more opaque than the fill.
                let edge=Self::rgba(c,(s.bar_opacity+0.2).max(0.6).min(1.0));
                let w=frame.w as usize;let h=frame.h as usize;
                for y in 0..h {for x in 0..w {if x==0||y==0||x+1==w||y+1==h {
                    let i=(y*w+x)*4;pixels[i..i+4].copy_from_slice(&edge);
                }}}
                self.upload(&self.bar,&pixels,frame.w,frame.h)?;
                self.last_bar=frame;
            }
            if !self.bar.mapped {self.conn.map_window(self.bar.id)?; self.bar.mapped=true;}
        } else if self.bar.mapped {self.conn.unmap_window(self.bar.id)?;self.bar.mapped=false;}
        if overlay_visible {
            let (c,opacity,cut)=if s.mode==Mode::ScreenTint {
                (color(&s.tint_color_name),s.tint_opacity,None)
            } else {(color(&s.spotlight_color_name),s.spotlight_opacity,Some(frame))};
            let bounds=Rect{x:0,y:0,w:self.last_root_size.0,h:self.last_root_size.1};
            if force||self.last_bar!=frame||!self.overlay.mapped {
                let pixels=Self::pixels(bounds.w,bounds.h,Self::rgba(c,opacity),cut,(0,0));
                self.upload(&self.overlay,&pixels,bounds.w,bounds.h)?;
            }
            if !self.overlay.mapped {self.conn.map_window(self.overlay.id)?;self.overlay.mapped=true;}
            // Keep the bar above the overlay. Overlay remains click-through.
            self.conn.configure_window(self.overlay.id,&ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
            if bar_visible {self.conn.configure_window(self.bar.id,&ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;}
        } else if self.overlay.mapped {self.conn.unmap_window(self.overlay.id)?;self.overlay.mapped=false;}
        self.conn.flush()?; Ok(())
    }
    fn save(&self){save("settings.json",&self.settings);save("profiles.json",&self.profiles);}
    fn handle_shortcut(&mut self,action:u32)->bool {
        match action {
            1=>self.hidden=!self.hidden,
            2=>self.toggle_lock(),
            3=>self.nudge(true), 4=>self.nudge(false), _=>return false
        }true
    }
    fn toggle_lock(&mut self) {
        let r=self.effective_bar();
        self.settings.is_locked=!self.settings.is_locked;
        if self.settings.is_locked {
            self.settings.locked_anchor_x=Some(r.x as f64+r.w as f64/2.0);
            self.settings.locked_anchor_y=Some(r.y as f64+r.h as f64/2.0);
        }
        self.save();
    }
    fn nudge(&mut self,up:bool) {
        if !self.settings.is_locked{return}
        let (x,y)=self.anchor();let delta=thickness(&self.settings)*if up {-1.0}else{1.0};
        let (nx,ny)=match self.settings.bar_orientation {Orientation::Horizontal=>(x,y+delta),Orientation::Vertical=>(x+delta,y)};
        let f=bar_rect(nx,ny,thickness(&self.settings),self.settings.bar_orientation,self.screen_for(x,y));
        self.settings.locked_anchor_x=Some(f.x as f64+f.w as f64/2.0);
        self.settings.locked_anchor_y=Some(f.y as f64+f.h as f64/2.0);
        self.save();
    }
    fn handle_command(&mut self,line:&str)->bool {
        let mut tokens=line.split_whitespace();let cmd=tokens.next().unwrap_or("");let arg=tokens.next().unwrap_or("");
        match cmd {
            "help"=>print_help(), "quit"|"exit"=>return true,
            "show"=>self.hidden=false, "hide"=>self.hidden=true, "toggle"=>self.hidden=!self.hidden,
            "lock"=>{if !self.settings.is_locked{self.toggle_lock()}},
            "unlock"=>{if self.settings.is_locked{self.toggle_lock()}},
            "nudge"=>self.nudge(arg=="up"),
            "mode"=> match arg {"off"=>self.settings.mode=Mode::Off,"bar"=>self.settings.mode=Mode::BarOnly,
                "spotlight"=>self.settings.mode=Mode::BarAndSpotlight,"tint"=>self.settings.mode=Mode::ScreenTint,_=>eprintln!("mode: off|bar|spotlight|tint")},
            "shape"=> match arg {"line"=>self.settings.bar_shape=Shape::Line,"ruler"=>self.settings.bar_shape=Shape::Ruler,_=>eprintln!("shape: line|ruler")},
            "orientation"=>match arg {"horizontal"=>self.settings.bar_orientation=Orientation::Horizontal,"vertical"=>self.settings.bar_orientation=Orientation::Vertical,_=>eprintln!("orientation: horizontal|vertical")},
            "tracking"=>match arg {"mouse"=>self.settings.tracking_source=Tracking::Mouse,"scroll"=>self.settings.tracking_source=Tracking::Scroll,
                "keyboard"=>self.settings.tracking_source=Tracking::Keyboard,_=>eprintln!("tracking: mouse|scroll|keyboard")},
            "size"=>if let Ok(v)=arg.parse::<f64>(){self.settings.font_reference_size=v.clamp(10.0,100.0)},
            "opacity"=>if let Ok(v)=arg.parse::<f64>(){self.settings.bar_opacity=v.clamp(0.1,0.9)},
            "spotlight-opacity"=>if let Ok(v)=arg.parse::<f64>(){self.settings.spotlight_opacity=v.clamp(0.1,0.9)},
            "tint-opacity"=>if let Ok(v)=arg.parse::<f64>(){self.settings.tint_opacity=v.clamp(0.1,0.9)},
            "color"=>self.settings.color_name=arg.into(),
            "spotlight-color"=>self.settings.spotlight_color_name=arg.into(),
            "tint-color"=>self.settings.tint_color_name=arg.into(),
            "profiles"=>{for p in builtins().iter().chain(self.profiles.iter()){println!("{}",p.name)}},
            "profile"=>{
                let name=line.strip_prefix("profile ").unwrap_or("").trim();
                if let Some(p)=builtins().into_iter().chain(self.profiles.iter().cloned()).find(|p|p.name.eq_ignore_ascii_case(name)) {
                    // Preserve app-level lock and lifecycle state.
                    let old=&self.settings;let mut new=p.settings;
                    new.is_locked=old.is_locked;new.locked_anchor_x=old.locked_anchor_x;
                    new.locked_anchor_y=old.locked_anchor_y;new.launch_at_login=old.launch_at_login;
                    new.per_app_enabled=old.per_app_enabled;new.enabled_bundle_ids=old.enabled_bundle_ids.clone();
                    self.settings=new;
                }else{eprintln!("Profile not found: {name}")}
            },
            "save-profile"=>{
                let name=line.strip_prefix("save-profile ").unwrap_or("").trim();
                if !name.is_empty() {
                    self.profiles.retain(|p|p.name!=name);
                    self.profiles.push(Profile{name:name.into(),settings:self.settings.clone(),is_built_in:false});
                }
            },
            "delete-profile"=>{let name=line.strip_prefix("delete-profile ").unwrap_or("").trim();self.profiles.retain(|p|p.name!=name);},
            "status"=>println!("{}",serde_json::to_string_pretty(&self.settings).unwrap_or_default()),
            ""=>(),_=>eprintln!("Unknown command. Type help."),
        }
        self.save();false
    }
    fn refresh_monitors(&mut self)->Result<bool,Box<dyn Error>> {
        let reply=self.conn.get_geometry(self.root)?.reply()?;
        let size=(reply.width as u32,reply.height as u32);
        let screens=Self::monitors(&self.conn,self.root,Rect{x:0,y:0,w:size.0,h:size.1});
        let changed=size!=self.last_root_size||screens!=self.screens;
        if changed {
            self.screens=screens;
            self.last_root_size=size;
            self.set_frame(&self.overlay,Rect{x:0,y:0,w:size.0,h:size.1})?;
        }
        Ok(changed)
    }
    fn run(mut self)->Result<(),Box<dyn Error>> {
        let (tx,rx)=mpsc::channel::<String>();
        thread::spawn(move||for l in io::stdin().lock().lines(){match l{Ok(line)=>if tx.send(line).is_err(){break},Err(_)=>break}});
        print_help();let mut previous=self.tracked;let mut last_monitor=Instant::now();
        loop {
            let mut dirty=false;let mut quit=false;
            while let Ok(line)=rx.try_recv(){quit|=self.handle_command(&line);dirty=true;}
            if quit{break}
            while let Some(event)=self.conn.poll_for_event()? {
                match event {
                    Event::KeyPress(e)=>{
                        let actions:Vec<u32>=self.keys.iter().filter(|(key,_)|*key==e.detail).map(|(_,a)|*a).collect();
                        for action in actions{dirty|=self.handle_shortcut(action);}
                        if self.settings.tracking_source==Tracking::Keyboard&&!self.settings.is_locked {
                            let step=thickness(&self.settings);
                            if let Some((_,a))=self.keys.iter().find(|(key,a)|*key==e.detail&&(*a==3||*a==4)) {
                                self.tracked.1+=if *a==3{-step}else{step};dirty=true;
                            }
                        }
                    },
                    Event::ButtonPress(e)=>{
                        if self.settings.tracking_source==Tracking::Scroll&&!self.settings.is_locked {
                            let d=match e.detail{4=>-thickness(&self.settings),5=>thickness(&self.settings),_=>0.0};
                            if self.settings.bar_orientation==Orientation::Horizontal{self.tracked.1+=d}else{self.tracked.0+=d}
                            dirty|=d!=0.0;
                        }
                    },
                    Event::Expose(_)=>dirty=true,
                    _=>(),
                }
            }
            if self.settings.tracking_source==Tracking::Mouse&&!self.settings.is_locked {
                self.tracked=self.pointer()?;
                if self.tracked!=previous {dirty=true;previous=self.tracked;}
            }
            if last_monitor.elapsed()>Duration::from_secs(2) {
                dirty|=self.refresh_monitors()?;last_monitor=Instant::now();
            }
            if dirty{self.render(true)?;}
            thread::sleep(Duration::from_millis(16));
        }
        self.save();Ok(())
    }
}
fn print_help() {
    println!("\nHighlight Bar Linux (X11)\n  Super+Shift+H   show/hide\n  Super+Shift+L   lock/unlock\n  Super+Shift+Up/Down  nudge when locked\n\nCommands (enter here):\n  mode off|bar|spotlight|tint\n  shape ruler|line          orientation horizontal|vertical\n  color Yellow|Green|Blue|Pink|Orange|Gray|#RRGGBB\n  spotlight-color NAME      tint-color NAME\n  opacity 0.1..0.9         spotlight-opacity 0.1..0.9\n  tint-opacity 0.1..0.9     size 10..100\n  tracking mouse|scroll|keyboard\n  lock|unlock|nudge up|nudge down|show|hide|toggle\n  profiles|profile NAME|save-profile NAME|delete-profile NAME\n  status|help|quit\n");
}
fn main()->Result<(),Box<dyn Error>> {
    if env::args().any(|a|a=="--help"||a=="-h"){print_help();return Ok(())}
    if env::var_os("WAYLAND_DISPLAY").is_some()&&env::var_os("DISPLAY").is_none() {
        return Err("Native Wayland is not supported; log in to an X11/Xorg session".into());
    }
    let app=XApp::new()?;app.run()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn geometry_clamped(){
        let s=Rect{x:0,y:0,w:1920,h:1080};
        assert_eq!(bar_rect(200.0,0.0,44.0,Orientation::Horizontal,s).y,0);
        assert_eq!(bar_rect(200.0,5000.0,44.0,Orientation::Horizontal,s).y,1036);
        assert_eq!(bar_rect(0.0,500.0,44.0,Orientation::Vertical,s).x,0);
    }
    #[test] fn palette(){assert_eq!(color("#1A2B3C"),[26,43,60]);}
    #[test] fn defaults(){let s:Settings=serde_json::from_str("{}").unwrap();assert_eq!(s.mode,Mode::BarOnly);}
}
