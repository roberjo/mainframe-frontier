       IDENTIFICATION DIVISION.
       PROGRAM-ID. TRNVALID.
       AUTHOR. FIRST FRONTIER BANK - CORE DEPOSITS.
      *================================================================*
      * TRNVALID - VALIDATE THE DAILY TRANSACTION FEED                 *
      *                                                                *
      * EDITS EVERY FEED RECORD. GOOD RECORDS ARE CONVERTED TO THE     *
      * INTERNAL TRANSACTION LAYOUT WITH A SIGNED PACKED AMOUNT (DEBITS*
      * NEGATIVE). BAD RECORDS ARE LISTED ON THE REJECTS REPORT.       *
      *                                                                *
      *   PARM     BUSINESS DATE YYYYMMDD                              *
      *   TRANIN   SORTED FEED        (TRANFEED LAYOUT, FB 80)  INPUT  *
      *   TRANOUT  VALID TRANSACTIONS (TRANREC LAYOUT,  FB 80)  OUTPUT *
      *   REJECTS  REJECTS REPORT     (132-BYTE PRINT LINES)    OUTPUT *
      *                                                                *
      *   RC  0  ALL RECORDS VALID                                     *
      *   RC  4  SOME RECORDS REJECTED (WITHIN TOLERANCE)              *
      *   RC  8  REJECT RATE ABOVE 5 PERCENT - STOP THE CYCLE          *
      *   RC 16  FILE OR PARM ERROR                                    *
      *                                                                *
      * EDITS (FIRST FAILURE WINS)                                     *
      *   V001 ACCOUNT ID MISSING      V004 INVALID TRANSACTION TYPE   *
      *   V002 INVALID TIMESTAMP       V005 AMOUNT NOT NUMERIC         *
      *   V003 NOT FOR BUSINESS DATE   V006 ZERO AMOUNT                *
      *================================================================*
       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT TRAN-IN     ASSIGN TO TRANIN
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-IN-STATUS.
           SELECT TRAN-OUT    ASSIGN TO TRANOUT
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-OUT-STATUS.
           SELECT REJECT-RPT  ASSIGN TO REJECTS
                              ORGANIZATION IS LINE SEQUENTIAL
                              FILE STATUS IS WS-RPT-STATUS.

       DATA DIVISION.
       FILE SECTION.
       FD  TRAN-IN
           RECORDING MODE IS F.
       COPY TRANFEED REPLACING ==:TF:== BY ==TF==.

       FD  TRAN-OUT
           RECORDING MODE IS F.
       COPY TRANREC REPLACING ==:TR:== BY ==TR==.

       FD  REJECT-RPT.
       01  RPT-LINE                    PIC X(132).

       WORKING-STORAGE SECTION.
       COPY BUSDATE.
       01  WS-IN-STATUS                PIC X(02).
       01  WS-OUT-STATUS               PIC X(02).
       01  WS-RPT-STATUS               PIC X(02).
       01  WS-EOF-SW                   PIC X(01) VALUE 'N'.
           88  END-OF-INPUT                      VALUE 'Y'.
       01  WS-TS-DATE-N                PIC 9(08).
       01  WS-REASON-IX                PIC 9(01) VALUE ZERO.

       01  WS-REASON-TABLE.
           05  FILLER  PIC X(34) VALUE 'V001ACCOUNT ID MISSING'.
           05  FILLER  PIC X(34) VALUE 'V002INVALID TIMESTAMP'.
           05  FILLER  PIC X(34) VALUE 'V003NOT FOR BUSINESS DATE'.
           05  FILLER  PIC X(34) VALUE 'V004INVALID TRANSACTION TYPE'.
           05  FILLER  PIC X(34) VALUE 'V005AMOUNT NOT NUMERIC'.
           05  FILLER  PIC X(34) VALUE 'V006ZERO AMOUNT'.
       01  WS-REASONS REDEFINES WS-REASON-TABLE.
           05  WS-REASON-ENTRY OCCURS 6 TIMES.
               10  WS-REASON-CODE      PIC X(04).
               10  WS-REASON-TEXT      PIC X(30).

       01  WS-COUNTERS.
           05  WS-READ-COUNT           PIC 9(09) COMP VALUE ZERO.
           05  WS-VALID-COUNT          PIC 9(09) COMP VALUE ZERO.
           05  WS-REJECT-COUNT         PIC 9(09) COMP VALUE ZERO.
           05  WS-REJECTS-BY-REASON    PIC 9(09) COMP OCCURS 6 TIMES.
       01  WS-CREDIT-TOTAL             PIC S9(13)V99 COMP-3 VALUE ZERO.
       01  WS-DEBIT-TOTAL              PIC S9(13)V99 COMP-3 VALUE ZERO.
       01  WS-REJECT-PCT               PIC 9(03)V99 VALUE ZERO.

       01  WS-PAGE-CONTROL.
           05  WS-PAGE-NO              PIC 9(04) COMP VALUE ZERO.
           05  WS-LINE-NO              PIC 9(04) COMP VALUE 99.
           05  WS-LINES-PER-PAGE       PIC 9(04) COMP VALUE 55.

       01  RPT-HEADING-1.
           05  FILLER                  PIC X(20)
                                       VALUE 'FIRST FRONTIER BANK'.
           05  FILLER                  PIC X(46)
               VALUE 'DAILY TRANSACTION VALIDATION - REJECTED ITEMS'.
           05  FILLER                  PIC X(15)
                                       VALUE 'BUSINESS DATE '.
           05  RH1-DATE                PIC 9999/99/99.
           05  FILLER                  PIC X(10) VALUE SPACES.
           05  FILLER                  PIC X(05) VALUE 'PAGE '.
           05  RH1-PAGE                PIC ZZZ9.
       01  RPT-HEADING-2.
           05  FILLER  PIC X(66) VALUE
               'ACCOUNT     TXN-ID        TYPE  TIMESTAMP       AMOUNT'.
           05  FILLER  PIC X(30) VALUE 'REASON'.
       01  RPT-HEADING-3.
           05  FILLER  PIC X(66) VALUE
               '----------  ------------  ----  --------------  ------'.
           05  FILLER  PIC X(35) VALUE
           '---------------------------------- '.
       01  RPT-DETAIL.
           05  RD-ACCT-ID              PIC X(10).
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RD-TXN-ID               PIC X(12).
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RD-TYPE                 PIC X(02).
           05  FILLER                  PIC X(04) VALUE SPACES.
           05  RD-TIMESTAMP            PIC X(14).
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RD-AMOUNT               PIC X(11).
           05  FILLER                  PIC X(07) VALUE SPACES.
           05  RD-REASON-CODE          PIC X(04).
           05  FILLER                  PIC X(01) VALUE SPACE.
           05  RD-REASON-TEXT          PIC X(30).
       01  RPT-TOTAL-LINE.
           05  RT-LABEL                PIC X(40).
           05  RT-VALUE                PIC ZZZ,ZZZ,ZZ9.
       01  RPT-AMOUNT-LINE.
           05  RA-LABEL                PIC X(40).
           05  RA-VALUE                PIC -Z,ZZZ,ZZZ,ZZZ,ZZ9.99.

       01  WS-EDIT-COUNT               PIC ZZZ,ZZZ,ZZ9.
       01  WS-EDIT-AMOUNT              PIC -Z,ZZZ,ZZZ,ZZZ,ZZ9.99.
       01  WS-EDIT-PCT                 PIC ZZ9.99.
       01  WS-IX                       PIC 9(02) COMP.

       PROCEDURE DIVISION.
       0000-MAINLINE.
           PERFORM 9800-GET-BUS-DATE
           PERFORM 1000-INITIALIZE
           PERFORM 2000-PROCESS-RECORD UNTIL END-OF-INPUT
           PERFORM 3000-TERMINATE
           STOP RUN.

       1000-INITIALIZE.
           INITIALIZE WS-REJECTS-BY-REASON (1) WS-REJECTS-BY-REASON (2)
                      WS-REJECTS-BY-REASON (3) WS-REJECTS-BY-REASON (4)
                      WS-REJECTS-BY-REASON (5) WS-REJECTS-BY-REASON (6)
           MOVE WS-BUS-DATE TO RH1-DATE
           OPEN INPUT  TRAN-IN
                OUTPUT TRAN-OUT
                       REJECT-RPT
           IF WS-IN-STATUS NOT = '00' OR WS-OUT-STATUS NOT = '00'
                                      OR WS-RPT-STATUS NOT = '00'
               DISPLAY 'TRNVALID: OPEN FAILED, STATUS IN='
                       WS-IN-STATUS ' OUT=' WS-OUT-STATUS
                       ' RPT=' WS-RPT-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           PERFORM 8000-READ-FEED.

       2000-PROCESS-RECORD.
           PERFORM 2100-EDIT-RECORD
           IF WS-REASON-IX = ZERO
               PERFORM 2200-WRITE-VALID
           ELSE
               PERFORM 2300-WRITE-REJECT
           END-IF
           PERFORM 8000-READ-FEED.

      *----------------------------------------------------------------*
      * EDITS ARE NESTED SO A NUMERIC FIELD IS NEVER COMPARED BEFORE   *
      * IT HAS PASSED ITS CLASS TEST (THAT WOULD BE AN S0C7).          *
      *----------------------------------------------------------------*
       2100-EDIT-RECORD.
           MOVE ZERO TO WS-REASON-IX
           IF TF-ACCT-ID = SPACES
               MOVE 1 TO WS-REASON-IX
           END-IF

           IF WS-REASON-IX = ZERO
               IF TF-TIMESTAMP IS NOT NUMERIC
                   MOVE 2 TO WS-REASON-IX
               ELSE
                   MOVE TF-TS-DATE TO WS-TS-DATE-N
                   IF FUNCTION TEST-DATE-YYYYMMDD(WS-TS-DATE-N)
                      NOT = ZERO
                       MOVE 2 TO WS-REASON-IX
                   ELSE
                       IF WS-TS-DATE-N NOT = WS-BUS-DATE
                           MOVE 3 TO WS-REASON-IX
                       END-IF
                   END-IF
               END-IF
           END-IF

           IF WS-REASON-IX = ZERO
               IF NOT TF-VALID-TYPE
                   MOVE 4 TO WS-REASON-IX
               END-IF
           END-IF

           IF WS-REASON-IX = ZERO
               IF TF-AMOUNT IS NOT NUMERIC
                   MOVE 5 TO WS-REASON-IX
               ELSE
                   IF TF-AMOUNT = ZERO
                       MOVE 6 TO WS-REASON-IX
                   END-IF
               END-IF
           END-IF.

       2200-WRITE-VALID.
           MOVE SPACES           TO TR-TRAN-REC
           MOVE TF-ACCT-ID       TO TR-ACCT-ID
           MOVE TF-TIMESTAMP     TO TR-TIMESTAMP
           MOVE TF-TXN-ID        TO TR-TXN-ID
           MOVE TF-TXN-TYPE      TO TR-TXN-TYPE
           MOVE TF-CHANNEL       TO TR-CHANNEL
           MOVE TF-DESCRIPTION   TO TR-DESCRIPTION
           IF TF-CREDIT-TYPE
               MOVE TF-AMOUNT TO TR-AMOUNT
               ADD TF-AMOUNT TO WS-CREDIT-TOTAL
           ELSE
               COMPUTE TR-AMOUNT = ZERO - TF-AMOUNT
               ADD TF-AMOUNT TO WS-DEBIT-TOTAL
           END-IF
           WRITE TR-TRAN-REC
           IF WS-OUT-STATUS NOT = '00'
               DISPLAY 'TRNVALID: WRITE TRANOUT FAILED, STATUS '
                       WS-OUT-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           ADD 1 TO WS-VALID-COUNT.

       2300-WRITE-REJECT.
           ADD 1 TO WS-REJECT-COUNT
           ADD 1 TO WS-REJECTS-BY-REASON (WS-REASON-IX)
           IF WS-LINE-NO >= WS-LINES-PER-PAGE
               PERFORM 7000-PAGE-HEADING
           END-IF
           MOVE TF-ACCT-ID       TO RD-ACCT-ID
           MOVE TF-TXN-ID        TO RD-TXN-ID
           MOVE TF-TXN-TYPE      TO RD-TYPE
           MOVE TF-TIMESTAMP     TO RD-TIMESTAMP
           MOVE TF-FEED-REC(39:11) TO RD-AMOUNT
           MOVE WS-REASON-CODE (WS-REASON-IX) TO RD-REASON-CODE
           MOVE WS-REASON-TEXT (WS-REASON-IX) TO RD-REASON-TEXT
           WRITE RPT-LINE FROM RPT-DETAIL
           ADD 1 TO WS-LINE-NO.

       3000-TERMINATE.
           IF WS-READ-COUNT > ZERO
               COMPUTE WS-REJECT-PCT ROUNDED =
                   WS-REJECT-COUNT * 100 / WS-READ-COUNT
           END-IF
           PERFORM 3100-REPORT-TOTALS
           CLOSE TRAN-IN TRAN-OUT REJECT-RPT

           DISPLAY 'TRNVALID - TRANSACTION VALIDATION FOR ' WS-BUS-DATE
           MOVE WS-READ-COUNT TO WS-EDIT-COUNT
           DISPLAY '  RECORDS READ ............ ' WS-EDIT-COUNT
           MOVE WS-VALID-COUNT TO WS-EDIT-COUNT
           DISPLAY '  RECORDS ACCEPTED ........ ' WS-EDIT-COUNT
           MOVE WS-REJECT-COUNT TO WS-EDIT-COUNT
           DISPLAY '  RECORDS REJECTED ........ ' WS-EDIT-COUNT
           MOVE WS-REJECT-PCT TO WS-EDIT-PCT
           DISPLAY '  REJECT RATE (PERCENT) ... ' WS-EDIT-PCT
           MOVE WS-CREDIT-TOTAL TO WS-EDIT-AMOUNT
           DISPLAY '  CREDITS ACCEPTED ........ ' WS-EDIT-AMOUNT
           MOVE WS-DEBIT-TOTAL TO WS-EDIT-AMOUNT
           DISPLAY '  DEBITS ACCEPTED ......... ' WS-EDIT-AMOUNT

           EVALUATE TRUE
               WHEN WS-REJECT-COUNT = ZERO
                   MOVE 0 TO RETURN-CODE
               WHEN WS-REJECT-PCT > 5
                   DISPLAY 'TRNVALID: REJECT RATE EXCEEDS 5 PERCENT -'
                           ' CYCLE MUST NOT CONTINUE'
                   MOVE 8 TO RETURN-CODE
               WHEN OTHER
                   MOVE 4 TO RETURN-CODE
           END-EVALUATE.

       3100-REPORT-TOTALS.
           PERFORM 7000-PAGE-HEADING
           MOVE 'CONTROL TOTALS' TO RPT-LINE
           WRITE RPT-LINE
           MOVE SPACES TO RPT-LINE
           WRITE RPT-LINE
           MOVE '  RECORDS READ' TO RT-LABEL
           MOVE WS-READ-COUNT TO RT-VALUE
           WRITE RPT-LINE FROM RPT-TOTAL-LINE
           MOVE '  RECORDS ACCEPTED' TO RT-LABEL
           MOVE WS-VALID-COUNT TO RT-VALUE
           WRITE RPT-LINE FROM RPT-TOTAL-LINE
           MOVE '  RECORDS REJECTED' TO RT-LABEL
           MOVE WS-REJECT-COUNT TO RT-VALUE
           WRITE RPT-LINE FROM RPT-TOTAL-LINE
           PERFORM VARYING WS-IX FROM 1 BY 1 UNTIL WS-IX > 6
               MOVE SPACES TO RT-LABEL
               STRING '    ' WS-REASON-CODE (WS-IX) ' '
                      WS-REASON-TEXT (WS-IX)
                      DELIMITED BY SIZE INTO RT-LABEL
               MOVE WS-REJECTS-BY-REASON (WS-IX) TO RT-VALUE
               WRITE RPT-LINE FROM RPT-TOTAL-LINE
           END-PERFORM
           MOVE '  CREDITS ACCEPTED' TO RA-LABEL
           MOVE WS-CREDIT-TOTAL TO RA-VALUE
           WRITE RPT-LINE FROM RPT-AMOUNT-LINE
           MOVE '  DEBITS ACCEPTED' TO RA-LABEL
           MOVE WS-DEBIT-TOTAL TO RA-VALUE
           WRITE RPT-LINE FROM RPT-AMOUNT-LINE.

       7000-PAGE-HEADING.
           ADD 1 TO WS-PAGE-NO
           MOVE WS-PAGE-NO TO RH1-PAGE
           IF WS-PAGE-NO > 1
               MOVE SPACES TO RPT-LINE
               WRITE RPT-LINE
           END-IF
           WRITE RPT-LINE FROM RPT-HEADING-1
           MOVE SPACES TO RPT-LINE
           WRITE RPT-LINE
           WRITE RPT-LINE FROM RPT-HEADING-2
           WRITE RPT-LINE FROM RPT-HEADING-3
           MOVE 4 TO WS-LINE-NO.

       8000-READ-FEED.
           READ TRAN-IN
               AT END
                   SET END-OF-INPUT TO TRUE
               NOT AT END
                   ADD 1 TO WS-READ-COUNT
           END-READ.

       COPY BUSDATEP.
