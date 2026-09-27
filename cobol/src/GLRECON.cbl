       IDENTIFICATION DIVISION.
       PROGRAM-ID. GLRECON.
       AUTHOR. FIRST FRONTIER BANK - FINANCE CONTROL.
      *================================================================*
      * GLRECON - GENERAL LEDGER RECONCILIATION OF THE POSTING CYCLE   *
      *                                                                *
      * PROVES THE NIGHT'S WORK BALANCES BEFORE STATEMENTS GO OUT:     *
      *                                                                *
      *   OPENING LEDGER + NET POSTINGS + INTEREST CREDITED            *
      *                                        = CLOSING LEDGER        *
      *   OPENING ACCRUAL + DAILY ACCRUAL - INTEREST CREDITED          *
      *                                        = CLOSING ACCRUAL       *
      *   ACCOUNTS ON OLD MASTER               = ACCOUNTS ON NEW MASTER*
      *                                                                *
      *   PARM     BUSINESS DATE YYYYMMDD                              *
      *   ACCTOLD  MASTER BEFORE THE CYCLE (0)  (ACCTREC)       INPUT  *
      *   ACCTNEW  MASTER AFTER THE CYCLE (+1)  (ACCTREC)       INPUT  *
      *   POSTLOG  POSTING JOURNAL              (POSTREC)       INPUT  *
      *   INTLOG   ACCRUAL JOURNAL              (INTREC)        INPUT  *
      *   SYSPRINT RECONCILIATION REPORT                        OUTPUT *
      *                                                                *
      *   RC  0  IN BALANCE                                            *
      *   RC  8  OUT OF BALANCE - DOWNSTREAM STEPS MUST NOT RUN        *
      *   RC 16  FILE OR PARM ERROR                                    *
      *================================================================*
       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT ACCT-OLD    ASSIGN TO ACCTOLD
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-FS.
           SELECT ACCT-NEW    ASSIGN TO ACCTNEW
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-FS.
           SELECT POST-LOG    ASSIGN TO POSTLOG
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-FS.
           SELECT INT-LOG     ASSIGN TO INTLOG
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-FS.
           SELECT RECON-RPT   ASSIGN TO SYSPRINT
                              ORGANIZATION IS LINE SEQUENTIAL
                              FILE STATUS IS WS-FS.

       DATA DIVISION.
       FILE SECTION.
       FD  ACCT-OLD
           RECORDING MODE IS F.
       COPY ACCTREC REPLACING ==:AR:== BY ==OM==.

       FD  ACCT-NEW
           RECORDING MODE IS F.
       COPY ACCTREC REPLACING ==:AR:== BY ==NM==.

       FD  POST-LOG
           RECORDING MODE IS F.
       COPY POSTREC REPLACING ==:PR:== BY ==PR==.

       FD  INT-LOG
           RECORDING MODE IS F.
       COPY INTREC REPLACING ==:IR:== BY ==IR==.

       FD  RECON-RPT.
       01  RPT-LINE                    PIC X(132).

       WORKING-STORAGE SECTION.
       COPY BUSDATE.
       01  WS-FS                       PIC X(02).
       01  WS-EOF-SW                   PIC X(01).
           88  END-OF-FILE                       VALUE 'Y'.

       01  WS-OLD-TOTALS.
           05  WS-OLD-COUNT            PIC 9(09) COMP VALUE ZERO.
           05  WS-OLD-BALANCE          PIC S9(15)V99 COMP-3 VALUE 0.
           05  WS-OLD-ACCRUAL          PIC S9(11)V9(6) COMP-3 VALUE 0.
       01  WS-NEW-TOTALS.
           05  WS-NEW-COUNT            PIC 9(09) COMP VALUE ZERO.
           05  WS-NEW-BALANCE          PIC S9(15)V99 COMP-3 VALUE 0.
           05  WS-NEW-ACCRUAL          PIC S9(11)V9(6) COMP-3 VALUE 0.
       01  WS-JOURNAL-TOTALS.
           05  WS-POSTED-COUNT         PIC 9(09) COMP VALUE ZERO.
           05  WS-REJECTED-COUNT       PIC 9(09) COMP VALUE ZERO.
           05  WS-NET-POSTINGS         PIC S9(15)V99 COMP-3 VALUE 0.
           05  WS-DAILY-ACCRUAL        PIC S9(11)V9(6) COMP-3 VALUE 0.
           05  WS-INT-CREDITED         PIC S9(15)V99 COMP-3 VALUE 0.

       01  WS-EXPECTED-BALANCE         PIC S9(15)V99 COMP-3.
       01  WS-BALANCE-DIFF             PIC S9(15)V99 COMP-3.
       01  WS-EXPECTED-ACCRUAL         PIC S9(11)V9(6) COMP-3.
       01  WS-ACCRUAL-DIFF             PIC S9(11)V9(6) COMP-3.
       01  WS-FAILURES                 PIC 9(02) VALUE ZERO.
       01  WS-RESULT                   PIC X(60).

       01  RPT-HEADING.
           05  FILLER                  PIC X(20)
                                       VALUE 'FIRST FRONTIER BANK'.
           05  FILLER                  PIC X(46)
               VALUE 'GENERAL LEDGER RECONCILIATION - POSTING CYCLE'.
           05  FILLER                  PIC X(15)
                                       VALUE 'BUSINESS DATE '.
           05  RH-DATE                 PIC 9999/99/99.
       01  RPT-AMOUNT-LINE.
           05  RA-LABEL                PIC X(44).
           05  RA-AMOUNT               PIC -ZZZ,ZZZ,ZZZ,ZZZ,ZZ9.99.
           05  FILLER                  PIC X(04) VALUE SPACES.
           05  RA-FLAG                 PIC X(24).
       01  RPT-ACCRUAL-LINE.
           05  RC-LABEL                PIC X(44).
           05  RC-AMOUNT               PIC -ZZZ,ZZZ,ZZ9.999999.
           05  FILLER                  PIC X(04) VALUE SPACES.
           05  RC-FLAG                 PIC X(24).
       01  RPT-COUNT-LINE.
           05  RN-LABEL                PIC X(44).
           05  RN-COUNT                PIC ZZZ,ZZZ,ZZZ,ZZ9.
           05  FILLER                  PIC X(12) VALUE SPACES.
           05  RN-FLAG                 PIC X(24).

       PROCEDURE DIVISION.
       0000-MAINLINE.
           PERFORM 9800-GET-BUS-DATE
           PERFORM 1000-SUM-OLD-MASTER
           PERFORM 1100-SUM-NEW-MASTER
           PERFORM 1200-SUM-POSTINGS
           PERFORM 1300-SUM-INTEREST
           PERFORM 2000-RECONCILE
           STOP RUN.

       1000-SUM-OLD-MASTER.
           OPEN INPUT ACCT-OLD
           PERFORM 8000-CHECK-OPEN
           MOVE 'N' TO WS-EOF-SW
           PERFORM UNTIL END-OF-FILE
               READ ACCT-OLD
                   AT END
                       SET END-OF-FILE TO TRUE
                   NOT AT END
                       ADD 1           TO WS-OLD-COUNT
                       ADD OM-BALANCE  TO WS-OLD-BALANCE
                       ADD OM-ACCR-INT TO WS-OLD-ACCRUAL
               END-READ
           END-PERFORM
           CLOSE ACCT-OLD.

       1100-SUM-NEW-MASTER.
           OPEN INPUT ACCT-NEW
           PERFORM 8000-CHECK-OPEN
           MOVE 'N' TO WS-EOF-SW
           PERFORM UNTIL END-OF-FILE
               READ ACCT-NEW
                   AT END
                       SET END-OF-FILE TO TRUE
                   NOT AT END
                       ADD 1           TO WS-NEW-COUNT
                       ADD NM-BALANCE  TO WS-NEW-BALANCE
                       ADD NM-ACCR-INT TO WS-NEW-ACCRUAL
               END-READ
           END-PERFORM
           CLOSE ACCT-NEW.

       1200-SUM-POSTINGS.
           OPEN INPUT POST-LOG
           PERFORM 8000-CHECK-OPEN
           MOVE 'N' TO WS-EOF-SW
           PERFORM UNTIL END-OF-FILE
               READ POST-LOG
                   AT END
                       SET END-OF-FILE TO TRUE
                   NOT AT END
                       IF PR-POSTED
                           ADD 1         TO WS-POSTED-COUNT
                           ADD PR-AMOUNT TO WS-NET-POSTINGS
                       ELSE
                           ADD 1         TO WS-REJECTED-COUNT
                       END-IF
               END-READ
           END-PERFORM
           CLOSE POST-LOG.

       1300-SUM-INTEREST.
           OPEN INPUT INT-LOG
           PERFORM 8000-CHECK-OPEN
           MOVE 'N' TO WS-EOF-SW
           PERFORM UNTIL END-OF-FILE
               READ INT-LOG
                   AT END
                       SET END-OF-FILE TO TRUE
                   NOT AT END
                       ADD IR-DAILY-ACCR TO WS-DAILY-ACCRUAL
                       ADD IR-CREDITED   TO WS-INT-CREDITED
               END-READ
           END-PERFORM
           CLOSE INT-LOG.

       2000-RECONCILE.
           COMPUTE WS-EXPECTED-BALANCE =
                   WS-OLD-BALANCE + WS-NET-POSTINGS + WS-INT-CREDITED
           COMPUTE WS-BALANCE-DIFF =
                   WS-NEW-BALANCE - WS-EXPECTED-BALANCE
           COMPUTE WS-EXPECTED-ACCRUAL =
                   WS-OLD-ACCRUAL + WS-DAILY-ACCRUAL - WS-INT-CREDITED
           COMPUTE WS-ACCRUAL-DIFF =
                   WS-NEW-ACCRUAL - WS-EXPECTED-ACCRUAL

           OPEN OUTPUT RECON-RPT
           MOVE WS-BUS-DATE TO RH-DATE
           WRITE RPT-LINE FROM RPT-HEADING
           MOVE SPACES TO RPT-LINE
           WRITE RPT-LINE

           MOVE 'LEDGER BALANCE' TO RPT-LINE
           WRITE RPT-LINE
           MOVE SPACES TO RA-FLAG
           MOVE '  OPENING LEDGER        (ACCTMAST 0)' TO RA-LABEL
           MOVE WS-OLD-BALANCE TO RA-AMOUNT
           WRITE RPT-LINE FROM RPT-AMOUNT-LINE
           MOVE '  + NET POSTINGS        (POSTLOG)' TO RA-LABEL
           MOVE WS-NET-POSTINGS TO RA-AMOUNT
           WRITE RPT-LINE FROM RPT-AMOUNT-LINE
           MOVE '  + INTEREST CREDITED   (INTLOG)' TO RA-LABEL
           MOVE WS-INT-CREDITED TO RA-AMOUNT
           WRITE RPT-LINE FROM RPT-AMOUNT-LINE
           MOVE '  = EXPECTED CLOSING LEDGER' TO RA-LABEL
           MOVE WS-EXPECTED-BALANCE TO RA-AMOUNT
           WRITE RPT-LINE FROM RPT-AMOUNT-LINE
           MOVE '  ACTUAL CLOSING LEDGER (ACCTMAST +1)' TO RA-LABEL
           MOVE WS-NEW-BALANCE TO RA-AMOUNT
           WRITE RPT-LINE FROM RPT-AMOUNT-LINE
           MOVE '  DIFFERENCE' TO RA-LABEL
           MOVE WS-BALANCE-DIFF TO RA-AMOUNT
           IF WS-BALANCE-DIFF = ZERO
               MOVE 'IN BALANCE' TO RA-FLAG
           ELSE
               MOVE '*** OUT OF BALANCE ***' TO RA-FLAG
               ADD 1 TO WS-FAILURES
           END-IF
           WRITE RPT-LINE FROM RPT-AMOUNT-LINE
           MOVE SPACES TO RPT-LINE
           WRITE RPT-LINE

           MOVE 'ACCRUED INTEREST' TO RPT-LINE
           WRITE RPT-LINE
           MOVE SPACES TO RC-FLAG
           MOVE '  OPENING ACCRUAL       (ACCTMAST 0)' TO RC-LABEL
           MOVE WS-OLD-ACCRUAL TO RC-AMOUNT
           WRITE RPT-LINE FROM RPT-ACCRUAL-LINE
           MOVE '  + ACCRUED TODAY       (INTLOG)' TO RC-LABEL
           MOVE WS-DAILY-ACCRUAL TO RC-AMOUNT
           WRITE RPT-LINE FROM RPT-ACCRUAL-LINE
           MOVE '  - INTEREST CREDITED   (INTLOG)' TO RC-LABEL
           MOVE WS-INT-CREDITED TO RC-AMOUNT
           WRITE RPT-LINE FROM RPT-ACCRUAL-LINE
           MOVE '  ACTUAL CLOSING ACCRUAL (ACCTMAST +1)' TO RC-LABEL
           MOVE WS-NEW-ACCRUAL TO RC-AMOUNT
           WRITE RPT-LINE FROM RPT-ACCRUAL-LINE
           MOVE '  DIFFERENCE' TO RC-LABEL
           MOVE WS-ACCRUAL-DIFF TO RC-AMOUNT
           IF WS-ACCRUAL-DIFF = ZERO
               MOVE 'IN BALANCE' TO RC-FLAG
           ELSE
               MOVE '*** OUT OF BALANCE ***' TO RC-FLAG
               ADD 1 TO WS-FAILURES
           END-IF
           WRITE RPT-LINE FROM RPT-ACCRUAL-LINE
           MOVE SPACES TO RPT-LINE
           WRITE RPT-LINE

           MOVE 'ACCOUNTS AND ITEMS' TO RPT-LINE
           WRITE RPT-LINE
           MOVE SPACES TO RN-FLAG
           MOVE '  ACCOUNTS ON OLD MASTER' TO RN-LABEL
           MOVE WS-OLD-COUNT TO RN-COUNT
           WRITE RPT-LINE FROM RPT-COUNT-LINE
           MOVE '  ACCOUNTS ON NEW MASTER' TO RN-LABEL
           MOVE WS-NEW-COUNT TO RN-COUNT
           IF WS-NEW-COUNT = WS-OLD-COUNT
               MOVE 'IN BALANCE' TO RN-FLAG
           ELSE
               MOVE '*** COUNT MISMATCH ***' TO RN-FLAG
               ADD 1 TO WS-FAILURES
           END-IF
           WRITE RPT-LINE FROM RPT-COUNT-LINE
           MOVE SPACES TO RN-FLAG
           MOVE '  TRANSACTIONS POSTED' TO RN-LABEL
           MOVE WS-POSTED-COUNT TO RN-COUNT
           WRITE RPT-LINE FROM RPT-COUNT-LINE
           MOVE '  TRANSACTIONS REJECTED BY POSTING' TO RN-LABEL
           MOVE WS-REJECTED-COUNT TO RN-COUNT
           WRITE RPT-LINE FROM RPT-COUNT-LINE
           MOVE SPACES TO RPT-LINE
           WRITE RPT-LINE

           IF WS-FAILURES = ZERO
               MOVE 'RESULT: CYCLE RECONCILED - RELEASE STATEMENTS'
                 TO WS-RESULT
               MOVE ZERO TO RETURN-CODE
           ELSE
               MOVE 'RESULT: *** RECONCILIATION FAILED - HOLD CYCLE ***'
                 TO WS-RESULT
               MOVE 8 TO RETURN-CODE
           END-IF
           WRITE RPT-LINE FROM WS-RESULT
           CLOSE RECON-RPT
           DISPLAY 'GLRECON - ' WS-RESULT.

       8000-CHECK-OPEN.
           IF WS-FS NOT = '00'
               DISPLAY 'GLRECON: OPEN FAILED, STATUS ' WS-FS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF.

       COPY BUSDATEP.
