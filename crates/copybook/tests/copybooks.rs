//! Parse the repository's real copybooks and decode records the COBOL wrote.

use std::path::PathBuf;

use frontier_copybook::*;

fn repo_copybook(name: &str) -> Copybook {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../cobol/copybooks")
        .join(format!("{name}.cpy"));
    let src = std::fs::read_to_string(&path).unwrap();
    parse(name, &src, &ParseOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Account 4000000001 from FFB.ACCTMAST.G0003V00 after the month-end cycle
/// (captured with `frontier ds print --hex`).
fn acctmast_record() -> Vec<u8> {
    hex(
        "34303030 30303030 30315355 53414E20 422E204A 41434B53 4F4E2020 20202020
         20202020 20202020 53413230 30333032 32310000 00049690 0C00215F 00000000
         0C000000 0000598D 32303236 30393330 00000C20 20202020 20202020 20202020
         20202020",
    )
}

#[test]
fn record_lengths_match_the_jcl() {
    for (name, len) in [
        ("ACCTREC", 100),
        ("ACCTLOAD", 100),
        ("TRANFEED", 80),
        ("TRANREC", 80),
        ("POSTREC", 100),
        ("INTREC", 50),
    ] {
        let book = repo_copybook(name);
        assert_eq!(book.records[0].size, len, "{name}");
    }
}

#[test]
fn tags_are_stripped_and_offsets_are_right() {
    let book = repo_copybook("ACCTREC");
    let rec = &book.records[0];
    assert_eq!(rec.name.as_deref(), Some("ACCT-REC"));
    let find = |n: &str| {
        rec.children
            .iter()
            .find(|c| c.name.as_deref() == Some(n))
            .unwrap()
    };
    assert_eq!((find("BALANCE").offset, find("BALANCE").size), (50, 7));
    assert_eq!((find("INT-RATE").offset, find("INT-RATE").size), (57, 3));
    assert_eq!((find("ACCR-INT").offset, find("ACCR-INT").size), (65, 7));
    assert_eq!(find("TXN-COUNT-MTD").offset, 80);
    assert_eq!(find("ACCT-TYPE").conditions.len(), 2);
}

#[test]
fn decodes_a_real_master_record() {
    let book = repo_copybook("ACCTREC");
    let json = decode(
        &book.records[0],
        &acctmast_record(),
        &DecodeOptions::default(),
    )
    .to_json();
    assert_eq!(json["ACCT-ID"], "4000000001");
    assert_eq!(json["CUST-NAME"], "SUSAN B. JACKSON");
    assert_eq!(json["OPEN-DATE"], 20030221);
    assert_eq!(json["BALANCE"], "4969.00");
    assert_eq!(json["INT-RATE"], "0.0215");
    assert_eq!(json["OD-LIMIT"], "0.00");
    assert_eq!(json["ACCR-INT"], "-0.000598");
    assert_eq!(json["LAST-ACTIVITY"], 20260930);
    assert_eq!(json["TXN-COUNT-MTD"], 0);
    assert!(json.get("FILLER").is_none());
    let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    assert_eq!(keys[0], "ACCT-ID", "fields keep layout order");
}

#[test]
fn flatten_reports_88_levels_and_errors() {
    let book = repo_copybook("ACCTREC");
    let mut rec = acctmast_record();
    let node = decode(&book.records[0], &rec, &DecodeOptions::default());
    let fields = flatten(&node);
    let t = fields.iter().find(|f| f.name == "ACCT-TYPE").unwrap();
    assert_eq!(t.conditions, vec!["SAVINGS"]);
    assert_eq!(
        fields
            .iter()
            .find(|f| f.name == "STATUS")
            .unwrap()
            .conditions,
        vec!["ACTIVE"]
    );

    // The S0C7 from Phase 1: 'ZZZZZZZ' over the packed balance.
    rec[50..57].copy_from_slice(b"ZZZZZZZ");
    let node = decode(&book.records[0], &rec, &DecodeOptions::default());
    let bad: Vec<_> = flatten(&node)
        .into_iter()
        .filter(|f| f.value.is_err())
        .collect();
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].name, "BALANCE");
    assert_eq!(bad[0].offset, 50);
    assert!(
        bad[0]
            .value
            .as_ref()
            .unwrap_err()
            .contains("5A5A5A5A5A5A5A")
    );
}

#[test]
fn multiple_records_and_redefines() {
    let book = repo_copybook("BUSDATE");
    let names: Vec<_> = book
        .records
        .iter()
        .map(|r| r.display_name().to_string())
        .collect();
    assert_eq!(names, ["WS-PARM", "WS-BUS-DATE", "WS-BUS-DATE-X"]);
    assert_eq!(book.records[0].size, 80);
    let x = &book.records[2];
    assert_eq!(x.redefines.as_deref(), Some("WS-BUS-DATE"));
    assert_eq!(x.size, 8);
}

#[test]
fn listing_shows_positions() {
    let listing = repo_copybook("ACCTREC").listing();
    assert!(
        listing.contains("ACCTREC ACCT-REC  LENGTH 100"),
        "{listing}"
    );
    let bal = listing.lines().find(|l| l.contains(" BALANCE ")).unwrap();
    assert!(
        bal.contains("S9(11)V99 COMP-3") && bal.contains("    51      7"),
        "{bal}"
    );
    assert!(listing.contains("SAVINGS"));
}

#[test]
fn schema_describes_decoded_json() {
    let book = repo_copybook("ACCTREC");
    let s = json_schema(&book.records[0]);
    assert_eq!(s["title"], "ACCT-REC");
    assert_eq!(s["properties"]["ACCT-ID"]["maxLength"], 10);
    assert_eq!(
        s["properties"]["BALANCE"]["pattern"],
        "^-?\\d{1,11}\\.\\d{2}$"
    );
    assert_eq!(s["properties"]["OPEN-DATE"]["maximum"], 99_999_999);
    assert_eq!(s["properties"]["BALANCE"]["x-cobol"]["offset"], 50);
    assert_eq!(
        s["properties"]["ACCT-TYPE"]["x-cobol"]["conditions"]["SAVINGS"][0],
        "'S'"
    );
}

#[test]
fn ebcdic_round_trip_preserves_packed_fields() {
    let book = repo_copybook("ACCTREC");
    let rec = acctmast_record();
    let e37 = Encoding::Ebcdic(CodePage::Cp037);
    let mut report = TranscodeReport::default();
    let host = transcode(
        &book.records[0],
        &rec,
        100,
        Encoding::Local,
        e37,
        &mut report,
    )
    .unwrap();
    assert_eq!(
        &host[..4],
        &[0xF4, 0xF0, 0xF0, 0xF0],
        "account id in EBCDIC digits"
    );
    assert_eq!(host[40], 0xE2, "'S' in EBCDIC");
    assert_eq!(&host[50..57], &rec[50..57], "packed balance untouched");
    assert_eq!(host[99], 0x40, "filler spaces");
    let json = decode(
        &book.records[0],
        &host,
        &DecodeOptions {
            encoding: e37,
            ..Default::default()
        },
    )
    .to_json();
    assert_eq!(json["CUST-NAME"], "SUSAN B. JACKSON");
    assert_eq!(json["BALANCE"], "4969.00");
    let back = transcode(
        &book.records[0],
        &host,
        100,
        e37,
        Encoding::Local,
        &mut report,
    )
    .unwrap();
    assert_eq!(back, rec);
    assert_eq!(report.invalid_numeric, 0);
}

#[test]
fn zoned_signs_are_converted_not_just_translated() {
    let src = "       01  R.\n           05  N  PIC S9(3).\n           05  S  PIC S9(3) SIGN LEADING SEPARATE.\n";
    let book = parse("Z", src, &ParseOptions::default()).unwrap();
    let local = b"12s-045".to_vec(); // -123, -45
    let e37 = Encoding::Ebcdic(CodePage::Cp037);
    let mut report = TranscodeReport::default();
    let host = transcode(
        &book.records[0],
        &local,
        7,
        Encoding::Local,
        e37,
        &mut report,
    )
    .unwrap();
    assert_eq!(host, [0xF1, 0xF2, 0xD3, 0x60, 0xF0, 0xF4, 0xF5]);
}

#[test]
fn comp5_byte_order_follows_platform() {
    let src =
        "       01  R.\n           05  B5  PIC S9(4) COMP-5.\n           05  B4  PIC S9(4) COMP.\n";
    let book = parse("B", src, &ParseOptions::default()).unwrap();
    let local = [0x02, 0x01, 0x01, 0x02]; // 258 native LE, 258 big-endian
    let json = decode(&book.records[0], &local, &DecodeOptions::default()).to_json();
    assert_eq!(
        (json["B5"].as_i64(), json["B4"].as_i64()),
        (Some(258), Some(258))
    );
    let mut report = TranscodeReport::default();
    let host = transcode(
        &book.records[0],
        &local,
        4,
        Encoding::Local,
        Encoding::Ebcdic(CodePage::Cp1047),
        &mut report,
    )
    .unwrap();
    assert_eq!(host, [0x01, 0x02, 0x01, 0x02]);
}

#[test]
fn occurs_and_depending_on() {
    let src = "
       01  ORDER-REC.
           05  ORDER-ID          PIC X(4).
           05  TOTALS            OCCURS 2 TIMES.
               10  TOT-AMT       PIC 9(3) COMP-3.
           05  LINE-COUNT        PIC 9.
           05  LINES  OCCURS 1 TO 5 TIMES DEPENDING ON LINE-COUNT.
               10  SKU           PIC X(3).
               10  QTY           PIC 99.
";
    let book = parse("O", src, &ParseOptions::default()).unwrap();
    let rec = &book.records[0];
    assert_eq!(rec.size, 4 + 2 * 2 + 1 + 5 * 5);
    let mut data = b"A001".to_vec();
    data.extend_from_slice(&[0x12, 0x3F, 0x45, 0x6F]);
    data.extend_from_slice(b"2AAA01BBB02");
    data.resize(rec.size, b' ');
    let json = decode(rec, &data, &DecodeOptions::default()).to_json();
    assert_eq!(json["TOTALS"][1]["TOT-AMT"], 456);
    assert_eq!(json["LINES"].as_array().unwrap().len(), 2);
    assert_eq!(json["LINES"][1]["SKU"], "BBB");
    let s = json_schema(rec);
    assert_eq!(s["properties"]["LINES"]["minItems"], 1);
    assert_eq!(
        s["properties"]["LINES"]["x-cobol"]["dependingOn"],
        "LINE-COUNT"
    );
}

/// One fixed-format line: sequence number (1-6), indicator (7), code (8-72), identification (73-80).
fn fixed(seq: u32, indicator: char, code: &str, ident: &str) -> String {
    format!("{seq:06}{indicator}{code:<65}{ident}\n")
}

#[test]
fn source_format_details() {
    // Sequence numbers, identification area, comments, `*>` comments, lower case,
    // explicit REPLACING, and a literal continued onto the next line.
    let src = [
        fixed(100, '*', " A comment line", ""),
        fixed(200, ' ', "01  cust-rec.", "CUST0001"),
        fixed(
            300,
            ' ',
            "    05  :P:-NAME     PIC X(40) VALUE 'FIRST FRONTIER BANK",
            "CUST0002",
        ),
        fixed(400, '-', "    ' OF THE WEST'.", "CUST0003"),
        fixed(
            500,
            ' ',
            "    05  :P:-FLAGS    PIC X.  *> trailing comment",
            "",
        ),
        fixed(
            600,
            ' ',
            "        88  :P:-VIP  VALUES 'V' 'P' THRU 'R'.",
            "",
        ),
        fixed(700, ' ', "    05  FILLER       PIC X(3).", ""),
    ]
    .concat();
    let opts = ParseOptions {
        replacing: vec![(":P:".into(), "CU".into())],
        ..Default::default()
    };
    let book = parse("CUST", &src, &opts).unwrap();
    let rec = &book.records[0];
    assert_eq!(rec.name.as_deref(), Some("CUST-REC"));
    assert_eq!(rec.children[0].name.as_deref(), Some("CU-NAME"));
    // A continued literal runs through column 72, then resumes after the quote.
    match &rec.children[0].value {
        Some(Literal::Text(t)) => {
            assert!(t.starts_with("FIRST FRONTIER BANK   "), "{t:?}");
            assert!(t.ends_with("  OF THE WEST"), "{t:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rec.children[1].conditions[0].values.len(), 2);
    assert_eq!(rec.size, 44);
}

#[test]
fn copybooks_starting_below_01_are_wrapped() {
    let book = parse(
        "PART",
        "           05  A  PIC X.\n           05  B  PIC 9 COMP-3.\n",
        &ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(book.records[0].display_name(), "PART");
    assert!(book.records[0].synthetic);
    assert_eq!(book.records[0].size, 2);
}

#[test]
fn reports_errors_with_line_numbers() {
    let e = parse(
        "E",
        "       01  R.\n           05  A  PIC X COMP-3.\n",
        &ParseOptions::default(),
    )
    .unwrap_err();
    assert_eq!(e.line, 2);
    assert!(e.message.contains("COMP-3"));
    let e = parse("E", "       01  R.\n           05  A.\n              10 B PIC X.\n           05  C  REDEFINES Z PIC X.\n", &ParseOptions::default()).unwrap_err();
    assert!(e.message.contains("REDEFINES target Z"));
    let e = parse("E", "       01  R.\n           05  N PIC 9.\n           05  T OCCURS 1 TO 3 DEPENDING ON N PIC X.\n           05  Z PIC X.\n", &ParseOptions::default()).unwrap_err();
    assert!(e.message.contains("must be the last"));
    assert!(
        parse(
            "E",
            "       01  R PIC X(3) PIC.\n",
            &ParseOptions::default()
        )
        .is_err()
    );
}
