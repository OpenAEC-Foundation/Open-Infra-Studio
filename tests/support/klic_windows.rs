use crate::klic_extra::{attachment_file, open_file};
use std::{ffi::OsStr, fs, os::windows::ffi::OsStrExt, path::Path, ptr, time::{Duration, Instant}};
use windows_sys::Win32::System::Registry::*;

fn wide(s: &OsStr) -> Vec<u16> { s.encode_wide().chain(Some(0)).collect() }

// HKCR wordt alleen binnen dit testproces omgeleid. De bestaande PDF-associatie
// van de gebruiker wordt niet gewijzigd. Ook bij een mislukte assertie opruimen.
struct Association { key: HKEY, name: Vec<u16> }

impl Association {
    fn new(viewer: &Path) -> Self {
        let name = wide(OsStr::new(&format!(r"Software\OpenInfraKlicTest-{}", std::process::id())));
        let mut key = ptr::null_mut();
        unsafe {
            assert_eq!(RegCreateKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, ptr::null(), 0,
                                      KEY_ALL_ACCESS, ptr::null(), &mut key, ptr::null_mut()), 0);
        }
        let association = Self { key, name };
        association.set(".pdf", "OpenInfra.KlicTest.Pdf");
        association.set(r"OpenInfra.KlicTest.Pdf\shell\open\command",
                        &format!("\"{}\" --viewer \"%1\"", viewer.display()));
        unsafe { assert_eq!(RegOverridePredefKey(HKEY_CLASSES_ROOT, key), 0); }
        association
    }

    fn set(&self, subkey: &str, value: &str) {
        let subkey = wide(OsStr::new(subkey));
        let value = wide(OsStr::new(value));
        unsafe {
            assert_eq!(RegSetKeyValueW(self.key, subkey.as_ptr(), ptr::null(), REG_SZ,
                                      value.as_ptr().cast(), (value.len() * 2) as u32), 0);
        }
    }
}

impl Drop for Association {
    fn drop(&mut self) {
        unsafe {
            assert_eq!(RegOverridePredefKey(HKEY_CLASSES_ROOT, ptr::null_mut()), 0);
            RegCloseKey(self.key);
            assert_eq!(RegDeleteTreeW(HKEY_CURRENT_USER, self.name.as_ptr()), 0);
            assert_eq!(RegDeleteKeyW(HKEY_CURRENT_USER, self.name.as_ptr()), 0);
        }
    }
}

fn pdf() -> Vec<u8> {
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in ["<< /Type /Catalog /Pages 2 0 R >>",
                         "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                         "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> >>"].iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n{object}\nendobj\n", i + 1).bytes());
    }
    let xref = bytes.len();
    bytes.extend(b"xref\n0 4\n0000000000 65535 f \n");
    for offset in &offsets[1..] { bytes.extend(format!("{offset:010} 00000 n \n").bytes()); }
    bytes.extend(format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").bytes());
    bytes
}

fn reaches_viewer(file: &Path) -> Result<(), String> {
    open_file(file)?;
    let viewed = file.with_extension("viewed");
    let executed = file.with_extension("executed");
    let start = Instant::now();
    while !viewed.exists() && !executed.exists() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(25));
    }
    if executed.exists() { return Err(format!("bijlage als programma uitgevoerd: {}", file.display())); }
    let received = fs::read_to_string(viewed).map_err(|e| format!("viewer ontving geen bestand: {e}"))?;
    if received != file.to_str().unwrap() { return Err(format!("ander pad ontvangen: {received}")); }
    Ok(())
}

#[test]
fn windows_document_association() {
    let dir = std::env::temp_dir().join(format!("oi-klic-native-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let probe = std::env::var_os("KLIC_DOCUMENT_PROBE").expect("start scripts/test_klic_attachments.py");
    let program = dir.join("controle.exe");
    fs::copy(&probe, &program).unwrap();
    assert!(std::process::Command::new(&program).status().unwrap().success());
    assert!(program.with_extension("executed").exists(), "positieve controle: echt uitvoerbaar PE-programma");
    let _association = Association::new(Path::new(&probe));
    let mut errors = Vec::new();

    // De vermomde bijlage gaat via de ZIP-route, precies zoals bij Openen.
    let delivery = dir.join("Levering.zip");
    {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(fs::File::create(&delivery).unwrap());
        zip.start_file("bronnen/KL1001/brief.pdf", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&fs::read(&probe).unwrap()).unwrap();
        zip.finish().unwrap();
    }
    let disguised = attachment_file(&delivery, "bronnen/KL1001/brief.pdf", &dir.join("cache")).unwrap();
    if let Err(e) = reaches_viewer(&disguised) { errors.push(format!("PE als PDF: {e}")); }

    // Geldige PDF in een geldige map met procenttekens, ook een letterlijke
    // omgevingsvariabelenaam. Controleer het volledige pad bij de echte viewer.
    let project = dir.join("Ontwerp 100% & Water (R&D)!^/Letterlijk %TEMP%");
    fs::create_dir_all(project.join("bronnen/KL1001")).unwrap();
    fs::write(project.join("bronnen/KL1001/brief.pdf"), pdf()).unwrap();
    let file = attachment_file(&project.join("GI_levering.xml"), "bronnen/KL1001/brief.pdf", &dir.join("out")).unwrap();
    if let Err(e) = reaches_viewer(&file) { errors.push(format!("PDF in %-map: {e}")); }

    assert!(attachment_file(&delivery, "bronnen/KL1001/brief.exe", &dir.join("cache")).is_err());
    assert!(open_file(&program).is_err());
    // Een fout van de documentassociatie moet als fout terugkomen.
    if open_file(&dir.join("ontbreekt.pdf")).is_ok() {
        errors.push("ontbrekend bestand niet als fout gemeld".into());
    }
    drop(_association);
    fs::remove_dir_all(&dir).unwrap();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    println!("PE als .exe uitvoerbaar; PE als .pdf alleen naar viewer; PDF in %-map naar viewer; exact pad behouden; .exe geweigerd; ontbrekend bestand geweigerd.");
}
