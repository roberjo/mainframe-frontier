       IDENTIFICATION DIVISION.
       PROGRAM-ID. STMTGEN.
       AUTHOR. FIRST FRONTIER BANK - CUSTOMER COMMUNICATIONS.
      *================================================================*
      * STMTGEN - DAILY ACCOUNT ACTIVITY STATEMENTS                    *
      *                                                                *
      * MATCHES THE NEW MASTER WITH THE POSTING JOURNAL (BOTH IN       *
      * ACCOUNT SEQUENCE) AND PRINTS A STATEMENT FOR EVERY ACCOUNT     *
      * WITH ACTIVITY: OPENING BALANCE, EACH ITEM (POSTED OR REFUSED), *
      * ANY INTEREST CREDITED, AND THE CLOSING BALANCE.                *
      *                                                                *
      *   PARM     BUSINESS DATE YYYYMMDD                              *
      *   ACCTIN   NEW MASTER (+1)      (ACCTREC, FB 100)       INPUT  *
      *   POSTLOG  POSTING JOURNAL      (POSTREC, FB 100)       INPUT  *
      *   STMTRPT  STATEMENTS           (132-BYTE PRINT LINES)  OUTPUT *
      *                                                                *
      *   RC  0  COMPLETE          RC 16  FILE OR PARM ERROR           *
      *================================================================*
       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT ACCT-IN     ASSIGN TO ACCTIN
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-ACCT-STATUS.
           SELECT POST-LOG    ASSIGN TO POSTLOG
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-LOG-STATUS.
           SELECT STMT-RPT    ASSIGN TO STMTRPT
                              ORGANIZATION IS LINE SEQUENTIAL
                              FILE STATUS IS WS-RPT-STATUS.

       DATA DIVISION.
       FILE SECTION.
       FD  ACCT-IN
           RECORDING MODE IS F.
       COPY ACCTREC REPLACING ==:AR:== BY ==AM==.

       FD  POST-LOG
           RECORDING MODE IS F.
       COPY POSTREC REPLACING ==:PR:== BY ==PR==.

       FD  STMT-RPT.
       01  RPT-LINE                    PIC X(132).

       WORKING-STORAGE SECTION.
       COPY BUSDATE.
       01  WS-ACCT-STATUS              PIC X(02).
       01  WS-LOG-STATUS               PIC X(02).
       01  WS-RPT-STATUS               PIC X(02).
       01  WS-MASTER-KEY               PIC X(10) VALUE LOW-VALUES.
       01  WS-POST-KEY                 PIC X(10) VALUE LOW-VALUES.
       01  WS-STMT-OPEN-SW             PIC X(01) VALUE 'N'.
           88  STATEMENT-OPEN                    VALUE 'Y'.
       01  WS-OPENING-BALANCE          PIC S9(11)V99 COMP-3.
       01  WS-RUNNING-BALANCE          PIC S9(11)V99 COMP-3.
       01  WS-INTEREST                 PIC S9(11)V99 COMP-3.

       01  WS-COUNTERS.
           05  WS-STATEMENTS           PIC 9(09) COMP VALUE ZERO.
           05  WS-ITEMS                PIC 9(09) COMP VALUE ZERO.
           05  WS-REFUSED-ITEMS        PIC 9(09) COMP VALUE ZERO.
           05  WS-SKIPPED-NOAC         PIC 9(09) COMP VALUE ZERO.
       01  WS-PAGE-CONTROL.
           05  WS-PAGE-NO              PIC 9(05) COMP VALUE ZERO.
           05  WS-LINE-NO              PIC 9(04) COMP VALUE 99.
           05  WS-LINES-PER-PAGE       PIC 9(04) COMP VALUE 60.

       01  RPT-PAGE-HEADING.
           05  FILLER                  PIC X(20)
                                       VALUE 'FIRST FRONTIER BANK'.
           05  FILLER                  PIC X(46)
                                       VALUE 'DAILY ACCOUNT STATEMENTS'.
           05  FILLER                  PIC X(15)
                                       VALUE 'BUSINESS DATE '.
           05  RP-DATE                 PIC 9999/99/99.
           05  FILLER                  PIC X(10) VALUE SPACES.
           05  FILLER                  PIC X(05) VALUE 'PAGE '.
           05  RP-PAGE                 PIC ZZZZ9.
       01  RPT-ACCOUNT-LINE.
           05  FILLER                  PIC X(08) VALUE 'ACCOUNT '.
           05  RA-ACCT-ID              PIC X(10).
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RA-NAME                 PIC X(30).
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RA-TYPE                 PIC X(08).
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RA-STATUS               PIC X(10).
       01  RPT-COLUMN-HEADING.
           05  FILLER  PIC X(13) VALUE '   TIME'.
           05  FILLER  PIC X(14) VALUE 'TXN-ID'.
           05  FILLER  PIC X(06) VALUE 'TYPE'.
           05  FILLER  PIC X(20) VALUE 'DESCRIPTION'.
           05  FILLER  PIC X(15) VALUE '         AMOUNT'.
           05  FILLER  PIC X(21) VALUE '              BALANCE'.
           05  FILLER  PIC X(06) VALUE '  NOTE'.
       01  RPT-BALANCE-LINE.
           05  FILLER                  PIC X(03) VALUE SPACES.
           05  RB-LABEL                PIC X(50).
           05  RB-AMOUNT               PIC ----,---,--9.99
                                       BLANK WHEN ZERO.
           05  FILLER                  PIC X(03) VALUE SPACES.
           05  RB-BALANCE              PIC ---,---,---,--9.99.
       01  RPT-ITEM-LINE.
           05  FILLER                  PIC X(03) VALUE SPACES.
           05  RI-TIME.
               10  RI-HH               PIC X(02).
               10  FILLER              PIC X(01) VALUE ':'.
               10  RI-MM               PIC X(02).
               10  FILLER              PIC X(01) VALUE ':'.
               10  RI-SS               PIC X(02).
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RI-TXN-ID               PIC X(12).
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RI-TYPE                 PIC X(02).
           05  FILLER                  PIC X(04) VALUE SPACES.
           05  RI-DESCRIPTION          PIC X(20).
           05  RI-AMOUNT               PIC ----,---,--9.99.
           05  FILLER                  PIC X(03) VALUE SPACES.
           05  RI-BALANCE              PIC ---,---,---,--9.99.
           05  FILLER                  PIC X(02) VALUE SPACES.
           05  RI-NOTE                 PIC X(14).
       01  RPT-TOTAL-LINE.
           05  RT-LABEL                PIC X(40).
           05  RT-VALUE                PIC ZZZ,ZZZ,ZZ9.

       01  WS-EDIT-COUNT               PIC ZZZ,ZZZ,ZZ9.

       PROCEDURE DIVISION.
       0000-MAINLINE.
           PERFORM 9800-GET-BUS-DATE
           PERFORM 1000-INITIALIZE
           PERFORM 2000-MATCH
               UNTIL WS-MASTER-KEY = HIGH-VALUES
                 AND WS-POST-KEY = HIGH-VALUES
           PERFORM 3000-TERMINATE
           STOP RUN.

       1000-INITIALIZE.
           MOVE WS-BUS-DATE TO RP-DATE
           OPEN INPUT  ACCT-IN POST-LOG
                OUTPUT STMT-RPT
           IF WS-ACCT-STATUS NOT = '00' OR WS-LOG-STATUS NOT = '00'
                                        OR WS-RPT-STATUS NOT = '00'
               DISPLAY 'STMTGEN: OPEN FAILED, STATUS ACCT='
                       WS-ACCT-STATUS ' LOG=' WS-LOG-STATUS
                       ' RPT=' WS-RPT-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           PERFORM 8100-READ-MASTER
           PERFORM 8200-READ-POSTING.

       2000-MATCH.
           EVALUATE TRUE
               WHEN WS-POST-KEY < WS-MASTER-KEY
      *            REFUSED FOR NO ACCOUNT: NOTHING TO STATE
                   ADD 1 TO WS-SKIPPED-NOAC
                   PERFORM 8200-READ-POSTING
               WHEN WS-POST-KEY = WS-MASTER-KEY
                   IF NOT STATEMENT-OPEN
                       PERFORM 2100-OPEN-STATEMENT
                   END-IF
                   PERFORM 2200-PRINT-ITEM
                   PERFORM 8200-READ-POSTING
               WHEN OTHER
                   IF STATEMENT-OPEN
                       PERFORM 2300-CLOSE-STATEMENT
                   END-IF
                   PERFORM 8100-READ-MASTER
           END-EVALUATE.

       2100-OPEN-STATEMENT.
           SET STATEMENT-OPEN TO TRUE
           ADD 1 TO WS-STATEMENTS
           IF PR-POSTED
               COMPUTE WS-OPENING-BALANCE = PR-BAL-AFTER - PR-AMOUNT
           ELSE
               MOVE PR-BAL-AFTER TO WS-OPENING-BALANCE
           END-IF
           MOVE WS-OPENING-BALANCE TO WS-RUNNING-BALANCE
           IF WS-LINE-NO + 8 > WS-LINES-PER-PAGE
               PERFORM 7000-PAGE-HEADING
           END-IF
           MOVE AM-ACCT-ID   TO RA-ACCT-ID
           MOVE AM-CUST-NAME TO RA-NAME
           IF AM-SAVINGS
               MOVE 'SAVINGS'  TO RA-TYPE
           ELSE
               MOVE 'CHECKING' TO RA-TYPE
           END-IF
           EVALUATE TRUE
               WHEN AM-ACTIVE  MOVE SPACES     TO RA-STATUS
               WHEN AM-FROZEN  MOVE '** FROZEN' TO RA-STATUS
               WHEN AM-CLOSED  MOVE '** CLOSED' TO RA-STATUS
           END-EVALUATE
           PERFORM 7100-WRITE-SEPARATOR
           WRITE RPT-LINE FROM RPT-ACCOUNT-LINE
           WRITE RPT-LINE FROM RPT-COLUMN-HEADING
           MOVE 'OPENING BALANCE' TO RB-LABEL
           MOVE ZERO TO RB-AMOUNT
           MOVE WS-OPENING-BALANCE TO RB-BALANCE
           WRITE RPT-LINE FROM RPT-BALANCE-LINE
           ADD 4 TO WS-LINE-NO.

       2200-PRINT-ITEM.
           IF WS-LINE-NO >= WS-LINES-PER-PAGE
               PERFORM 7000-PAGE-HEADING
           END-IF
           ADD 1 TO WS-ITEMS
           MOVE PR-TS-TIME(1:2)  TO RI-HH
           MOVE PR-TS-TIME(3:2)  TO RI-MM
           MOVE PR-TS-TIME(5:2)  TO RI-SS
           MOVE PR-TXN-ID        TO RI-TXN-ID
           MOVE PR-TXN-TYPE      TO RI-TYPE
           MOVE PR-DESCRIPTION   TO RI-DESCRIPTION
           MOVE PR-AMOUNT        TO RI-AMOUNT
           IF PR-POSTED
               MOVE PR-BAL-AFTER TO WS-RUNNING-BALANCE
               MOVE SPACES TO RI-NOTE
           ELSE
               ADD 1 TO WS-REFUSED-ITEMS
               STRING 'REFUSED ' PR-REASON DELIMITED BY SIZE
                   INTO RI-NOTE
           END-IF
           MOVE WS-RUNNING-BALANCE TO RI-BALANCE
           WRITE RPT-LINE FROM RPT-ITEM-LINE
           ADD 1 TO WS-LINE-NO.

       2300-CLOSE-STATEMENT.
           MOVE 'N' TO WS-STMT-OPEN-SW
           COMPUTE WS-INTEREST = AM-BALANCE - WS-RUNNING-BALANCE
           IF WS-INTEREST NOT = ZERO
               MOVE 'INTEREST CREDITED' TO RB-LABEL
               MOVE WS-INTEREST TO RB-AMOUNT
               MOVE AM-BALANCE TO RB-BALANCE
               WRITE RPT-LINE FROM RPT-BALANCE-LINE
               ADD 1 TO WS-LINE-NO
           END-IF
           MOVE 'CLOSING BALANCE' TO RB-LABEL
           MOVE ZERO TO RB-AMOUNT
           MOVE AM-BALANCE TO RB-BALANCE
           WRITE RPT-LINE FROM RPT-BALANCE-LINE
           MOVE SPACES TO RPT-LINE
           WRITE RPT-LINE
           ADD 2 TO WS-LINE-NO.

       3000-TERMINATE.
           PERFORM 7000-PAGE-HEADING
           MOVE 'STATEMENT RUN TOTALS' TO RPT-LINE
           WRITE RPT-LINE
           MOVE '  STATEMENTS PRODUCED' TO RT-LABEL
           MOVE WS-STATEMENTS TO RT-VALUE
           WRITE RPT-LINE FROM RPT-TOTAL-LINE
           MOVE '  ITEMS PRINTED' TO RT-LABEL
           MOVE WS-ITEMS TO RT-VALUE
           WRITE RPT-LINE FROM RPT-TOTAL-LINE
           MOVE '  ITEMS REFUSED AT POSTING' TO RT-LABEL
           MOVE WS-REFUSED-ITEMS TO RT-VALUE
           WRITE RPT-LINE FROM RPT-TOTAL-LINE
           MOVE '  ITEMS FOR UNKNOWN ACCOUNTS' TO RT-LABEL
           MOVE WS-SKIPPED-NOAC TO RT-VALUE
           WRITE RPT-LINE FROM RPT-TOTAL-LINE
           CLOSE ACCT-IN POST-LOG STMT-RPT

           DISPLAY 'STMTGEN - STATEMENTS FOR ' WS-BUS-DATE
           MOVE WS-STATEMENTS TO WS-EDIT-COUNT
           DISPLAY '  STATEMENTS PRODUCED ..... ' WS-EDIT-COUNT
           MOVE WS-ITEMS TO WS-EDIT-COUNT
           DISPLAY '  ITEMS PRINTED ........... ' WS-EDIT-COUNT
           MOVE WS-PAGE-NO TO WS-EDIT-COUNT
           DISPLAY '  PAGES ................... ' WS-EDIT-COUNT
           MOVE ZERO TO RETURN-CODE.

       7000-PAGE-HEADING.
           ADD 1 TO WS-PAGE-NO
           MOVE WS-PAGE-NO TO RP-PAGE
           IF WS-PAGE-NO > 1
               MOVE SPACES TO RPT-LINE
               WRITE RPT-LINE
           END-IF
           WRITE RPT-LINE FROM RPT-PAGE-HEADING
           MOVE SPACES TO RPT-LINE
           WRITE RPT-LINE
           MOVE 2 TO WS-LINE-NO.

       7100-WRITE-SEPARATOR.
           MOVE ALL '-' TO RPT-LINE
           WRITE RPT-LINE.

       8100-READ-MASTER.
           READ ACCT-IN
               AT END
                   MOVE HIGH-VALUES TO WS-MASTER-KEY
               NOT AT END
                   MOVE AM-ACCT-ID TO WS-MASTER-KEY
           END-READ.

       8200-READ-POSTING.
           READ POST-LOG
               AT END
                   MOVE HIGH-VALUES TO WS-POST-KEY
               NOT AT END
                   MOVE PR-ACCT-ID TO WS-POST-KEY
           END-READ.

       COPY BUSDATEP.
