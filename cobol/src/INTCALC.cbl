       IDENTIFICATION DIVISION.
       PROGRAM-ID. INTCALC.
       AUTHOR. FIRST FRONTIER BANK - CORE DEPOSITS.
      *================================================================*
      * INTCALC - DAILY INTEREST ACCRUAL AND MONTH-END CREDIT          *
      *                                                                *
      * ACTIVE SAVINGS ACCOUNTS WITH A POSITIVE BALANCE ACCRUE         *
      *     BALANCE * ANNUAL RATE / 365     (ACTUAL/365 FIXED)         *
      * TO SIX DECIMAL PLACES. ON THE LAST DAY OF THE MONTH THE        *
      * ACCRUAL IS CREDITED, ROUNDED TO THE CENT; THE SUB-CENT         *
      * RESIDUAL STAYS IN THE ACCRUAL SO NOTHING IS LOST TO ROUNDING.  *
      * MONTH-TO-DATE TRANSACTION COUNTS RESET AT MONTH END.           *
      *                                                                *
      *   PARM     BUSINESS DATE YYYYMMDD                              *
      *   ACCTIN   POSTED MASTER        (ACCTREC, FB 100)       INPUT  *
      *   ACCTOUT  NEW MASTER GEN (+1)  (ACCTREC, FB 100)       OUTPUT *
      *   INTLOG   ACCRUAL JOURNAL      (INTREC,  FB 50)        OUTPUT *
      *                                                                *
      *   RC  0  COMPLETE          RC 16  FILE OR PARM ERROR           *
      *================================================================*
       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT ACCT-IN     ASSIGN TO ACCTIN
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-IN-STATUS.
           SELECT ACCT-OUT    ASSIGN TO ACCTOUT
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-OUT-STATUS.
           SELECT INT-LOG     ASSIGN TO INTLOG
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-LOG-STATUS.

       DATA DIVISION.
       FILE SECTION.
       FD  ACCT-IN
           RECORDING MODE IS F.
       01  ACCT-IN-REC                 PIC X(100).

       FD  ACCT-OUT
           RECORDING MODE IS F.
       01  ACCT-OUT-REC                PIC X(100).

       FD  INT-LOG
           RECORDING MODE IS F.
       COPY INTREC REPLACING ==:IR:== BY ==IR==.

       WORKING-STORAGE SECTION.
       COPY ACCTREC REPLACING ==:AR:== BY ==WM==.
       COPY BUSDATE.

       01  WS-IN-STATUS                PIC X(02).
       01  WS-OUT-STATUS               PIC X(02).
       01  WS-LOG-STATUS               PIC X(02).
       01  WS-EOF-SW                   PIC X(01) VALUE 'N'.
           88  END-OF-INPUT                      VALUE 'Y'.

       01  WS-NEXT-DAY                 PIC 9(08).
       01  WS-NEXT-DAY-X REDEFINES WS-NEXT-DAY.
           05  FILLER                  PIC X(06).
           05  WS-NEXT-DAY-DD          PIC X(02).
       01  WS-MONTH-END-SW             PIC X(01) VALUE 'N'.
           88  MONTH-END                         VALUE 'Y'.

       01  WS-DAILY-ACCRUAL            PIC S9(07)V9(06) COMP-3.
       01  WS-CREDIT                   PIC S9(07)V99    COMP-3.
       01  WS-BALANCE-BASIS            PIC S9(11)V99    COMP-3.

       01  WS-COUNTERS.
           05  WS-READ-COUNT           PIC 9(09) COMP VALUE ZERO.
           05  WS-ACCRUING-COUNT       PIC 9(09) COMP VALUE ZERO.
           05  WS-CREDITED-COUNT       PIC 9(09) COMP VALUE ZERO.
       01  WS-TOTAL-ACCRUED            PIC S9(11)V9(06) COMP-3
                                                     VALUE ZERO.
       01  WS-TOTAL-CREDITED           PIC S9(13)V99    COMP-3
                                                     VALUE ZERO.

       01  WS-EDIT-COUNT               PIC ZZZ,ZZZ,ZZ9.
       01  WS-EDIT-ACCRUAL             PIC -ZZZ,ZZZ,ZZ9.999999.
       01  WS-EDIT-AMOUNT              PIC -Z,ZZZ,ZZZ,ZZZ,ZZ9.99.

       PROCEDURE DIVISION.
       0000-MAINLINE.
           PERFORM 9800-GET-BUS-DATE
           PERFORM 1000-INITIALIZE
           PERFORM 2000-PROCESS-ACCOUNT UNTIL END-OF-INPUT
           PERFORM 3000-TERMINATE
           STOP RUN.

       1000-INITIALIZE.
           COMPUTE WS-NEXT-DAY = FUNCTION DATE-OF-INTEGER(
                   FUNCTION INTEGER-OF-DATE(WS-BUS-DATE) + 1)
           IF WS-NEXT-DAY-DD = '01'
               SET MONTH-END TO TRUE
           END-IF
           OPEN INPUT  ACCT-IN
                OUTPUT ACCT-OUT INT-LOG
           IF WS-IN-STATUS NOT = '00' OR WS-OUT-STATUS NOT = '00'
                                      OR WS-LOG-STATUS NOT = '00'
               DISPLAY 'INTCALC: OPEN FAILED, STATUS IN=' WS-IN-STATUS
                       ' OUT=' WS-OUT-STATUS ' LOG=' WS-LOG-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           PERFORM 8000-READ-MASTER.

       2000-PROCESS-ACCOUNT.
           MOVE ZERO TO WS-DAILY-ACCRUAL WS-CREDIT
           MOVE WM-BALANCE TO WS-BALANCE-BASIS

           IF WM-SAVINGS AND WM-ACTIVE
              AND WM-BALANCE > ZERO AND WM-INT-RATE > ZERO
               COMPUTE WS-DAILY-ACCRUAL ROUNDED =
                       WM-BALANCE * WM-INT-RATE / 365
               ADD WS-DAILY-ACCRUAL TO WM-ACCR-INT
               ADD WS-DAILY-ACCRUAL TO WS-TOTAL-ACCRUED
               ADD 1 TO WS-ACCRUING-COUNT
           END-IF

           IF MONTH-END
               IF WM-ACCR-INT > ZERO
                   COMPUTE WS-CREDIT ROUNDED = WM-ACCR-INT
                   ADD WS-CREDIT TO WM-BALANCE
                   SUBTRACT WS-CREDIT FROM WM-ACCR-INT
                   ADD WS-CREDIT TO WS-TOTAL-CREDITED
                   ADD 1 TO WS-CREDITED-COUNT
               END-IF
               MOVE ZERO TO WM-TXN-COUNT-MTD
           END-IF

           IF WS-DAILY-ACCRUAL NOT = ZERO OR WS-CREDIT NOT = ZERO
               MOVE SPACES           TO IR-INT-REC
               MOVE WM-ACCT-ID       TO IR-ACCT-ID
               MOVE WS-BUS-DATE      TO IR-BUS-DATE
               MOVE WS-BALANCE-BASIS TO IR-BAL-BASIS
               MOVE WM-INT-RATE      TO IR-RATE
               MOVE WS-DAILY-ACCRUAL TO IR-DAILY-ACCR
               MOVE WS-CREDIT        TO IR-CREDITED
               WRITE IR-INT-REC
           END-IF

           WRITE ACCT-OUT-REC FROM WM-ACCT-REC
           IF WS-OUT-STATUS NOT = '00'
               DISPLAY 'INTCALC: WRITE ACCTOUT FAILED, STATUS '
                       WS-OUT-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           PERFORM 8000-READ-MASTER.

       3000-TERMINATE.
           CLOSE ACCT-IN ACCT-OUT INT-LOG
           DISPLAY 'INTCALC - INTEREST ACCRUAL FOR ' WS-BUS-DATE
           IF MONTH-END
               DISPLAY '  MONTH END: ACCRUED INTEREST CREDITED'
           END-IF
           MOVE WS-READ-COUNT TO WS-EDIT-COUNT
           DISPLAY '  ACCOUNTS READ ........... ' WS-EDIT-COUNT
           MOVE WS-ACCRUING-COUNT TO WS-EDIT-COUNT
           DISPLAY '  ACCOUNTS ACCRUING ....... ' WS-EDIT-COUNT
           MOVE WS-TOTAL-ACCRUED TO WS-EDIT-ACCRUAL
           DISPLAY '  INTEREST ACCRUED TODAY .. ' WS-EDIT-ACCRUAL
           MOVE WS-CREDITED-COUNT TO WS-EDIT-COUNT
           DISPLAY '  ACCOUNTS CREDITED ....... ' WS-EDIT-COUNT
           MOVE WS-TOTAL-CREDITED TO WS-EDIT-AMOUNT
           DISPLAY '  INTEREST CREDITED ....... ' WS-EDIT-AMOUNT
           MOVE ZERO TO RETURN-CODE.

       8000-READ-MASTER.
           READ ACCT-IN INTO WM-ACCT-REC
               AT END
                   SET END-OF-INPUT TO TRUE
               NOT AT END
                   ADD 1 TO WS-READ-COUNT
           END-READ.

       COPY BUSDATEP.
