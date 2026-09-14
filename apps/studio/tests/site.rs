use harness_browser::{BrowserLimits, ControlledBrowser};
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[test]
fn public_demo_runs_without_backend_and_responds_to_controls() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut browser = ControlledBrowser::launch_isolated(BrowserLimits::default()).unwrap();
    browser.set_viewport(1440, 1000).unwrap();
    let path = root
        .join("site/index.html")
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .to_owned();
    browser.navigate(&format!("file:///{path}")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if browser
            .eval("typeof snapshot!=='undefined' && snapshot?.status==='Completed'")
            .unwrap_or_default()
            == serde_json::json!(true)
        {
            break;
        }
        assert!(Instant::now() < deadline, "Demo did not complete");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(browser.eval("document.querySelector('#ambient-embers').width>0 && getComputedStyle(document.querySelector('#ambient-embers')).display!=='none'").unwrap(),true);
    browser.eval("window.dispatchEvent(new PointerEvent('pointermove',{clientX:200,clientY:200,pointerType:'mouse'}))").unwrap();
    browser.click("#prod-details").unwrap();
    assert_eq!(
        browser
            .eval("document.querySelector('#prod-details').getAttribute('aria-pressed')==='true'")
            .unwrap(),
        true
    );
    browser.click("#prod-details").unwrap();
    std::fs::write(root.join("site/preview.png"), browser.screenshot().unwrap()).unwrap();
    browser.click("#demo-run").unwrap();
    browser.click("#demo-stop").unwrap();
    assert_eq!(browser.eval("snapshot.status==='Stopped'").unwrap(), true);
    browser.set_viewport(390, 844).unwrap();
    assert_eq!(
        browser
            .eval("document.documentElement.scrollWidth<=innerWidth")
            .unwrap(),
        true
    );
}
