       IDENTIFICATION DIVISION.
       PROGRAM-ID. ACCTLOAD.
       AUTHOR. FIRST FRONTIER BANK - CORE DEPOSITS.
      *================================================================*
      * ACCTLOAD - INITIAL LOAD OF THE ACCOUNT MASTER                  *
      *                                                                *
      * CONVERTS THE LEGACY CONVERSION FEED TO THE MASTER LAYOUT:      *
      * DISPLAY NUMERICS BECOME PACKED DECIMAL, AND THE INPUT MUST BE  *
      * IN ASCENDING ACCOUNT SEQUENCE (THE NIGHTLY POSTING RUN DEPENDS *
      * ON IT).                                                        *
      *                                                                *
      *   ACCTIN   CONVERSION FEED  (ACCTLOAD LAYOUT, FB 100)   INPUT  *
      *   ACCTOUT  ACCOUNT MASTER   (ACCTREC LAYOUT,  FB 100)   OUTPUT *
      *                                                                *
      *   RC  0  LOADED                                                *
      *   RC 12  INPUT OUT OF SEQUENCE OR DUPLICATE ACCOUNT            *
      *   RC 16  FILE ERROR                                            *
      *================================================================*
       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT ACCT-IN   ASSIGN TO ACCTIN
                            ORGANIZATION IS SEQUENTIAL
                            FILE STATUS IS WS-IN-STATUS.
           SELECT ACCT-OUT  ASSIGN TO ACCTOUT
                            ORGANIZATION IS SEQUENTIAL
                            FILE STATUS IS WS-OUT-STATUS.

       DATA DIVISION.
       FILE SECTION.
       FD  ACCT-IN
           RECORDING MODE IS F.
       COPY ACCTLOAD REPLACING ==:AL:== BY ==AL==.

       FD  ACCT-OUT
           RECORDING MODE IS F.
       COPY ACCTREC REPLACING ==:AR:== BY ==AM==.

       WORKING-STORAGE SECTION.
       01  WS-IN-STATUS                PIC X(02).
       01  WS-OUT-STATUS               PIC X(02).
       01  WS-EOF-SW                   PIC X(01) VALUE 'N'.
           88  END-OF-INPUT                      VALUE 'Y'.
       01  WS-PREV-ACCT-ID             PIC X(10) VALUE LOW-VALUES.

       01  WS-COUNTERS.
           05  WS-READ-COUNT           PIC 9(09) COMP VALUE ZERO.
           05  WS-CHECKING-COUNT       PIC 9(09) COMP VALUE ZERO.
           05  WS-SAVINGS-COUNT        PIC 9(09) COMP VALUE ZERO.
       01  WS-TOTAL-BALANCE            PIC S9(15)V99 COMP-3 VALUE ZERO.

       01  WS-EDIT-COUNT               PIC ZZZ,ZZZ,ZZ9.
       01  WS-EDIT-AMOUNT              PIC -ZZZ,ZZZ,ZZZ,ZZZ,ZZ9.99.

       PROCEDURE DIVISION.
       0000-MAINLINE.
           PERFORM 1000-INITIALIZE
           PERFORM 2000-LOAD-ACCOUNT UNTIL END-OF-INPUT
           PERFORM 3000-TERMINATE
           STOP RUN.

       1000-INITIALIZE.
           OPEN INPUT ACCT-IN
           IF WS-IN-STATUS NOT = '00'
               DISPLAY 'ACCTLOAD: OPEN ACCTIN FAILED, STATUS '
                       WS-IN-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           OPEN OUTPUT ACCT-OUT
           IF WS-OUT-STATUS NOT = '00'
               DISPLAY 'ACCTLOAD: OPEN ACCTOUT FAILED, STATUS '
                       WS-OUT-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           PERFORM 8000-READ-INPUT.

       2000-LOAD-ACCOUNT.
           IF AL-ACCT-ID NOT > WS-PREV-ACCT-ID
               DISPLAY 'ACCTLOAD: SEQUENCE ERROR AT RECORD '
                       WS-READ-COUNT ' ACCOUNT ' AL-ACCT-ID
               MOVE 12 TO RETURN-CODE
               PERFORM 9000-CLOSE-FILES
               STOP RUN
           END-IF
           MOVE AL-ACCT-ID TO WS-PREV-ACCT-ID

           MOVE SPACES             TO AM-ACCT-REC
           MOVE AL-ACCT-ID         TO AM-ACCT-ID
           MOVE AL-CUST-NAME       TO AM-CUST-NAME
           MOVE AL-ACCT-TYPE       TO AM-ACCT-TYPE
           MOVE AL-STATUS          TO AM-STATUS
           MOVE AL-OPEN-DATE       TO AM-OPEN-DATE
           MOVE AL-BALANCE         TO AM-BALANCE
           MOVE AL-INT-RATE        TO AM-INT-RATE
           MOVE AL-OD-LIMIT        TO AM-OD-LIMIT
           MOVE ZERO               TO AM-ACCR-INT
           MOVE AL-OPEN-DATE       TO AM-LAST-ACTIVITY
           MOVE ZERO               TO AM-TXN-COUNT-MTD

           WRITE AM-ACCT-REC
           IF WS-OUT-STATUS NOT = '00'
               DISPLAY 'ACCTLOAD: WRITE FAILED, STATUS ' WS-OUT-STATUS
               MOVE 16 TO RETURN-CODE
               PERFORM 9000-CLOSE-FILES
               STOP RUN
           END-IF

           ADD AM-BALANCE TO WS-TOTAL-BALANCE
           IF AM-SAVINGS
               ADD 1 TO WS-SAVINGS-COUNT
           ELSE
               ADD 1 TO WS-CHECKING-COUNT
           END-IF
           PERFORM 8000-READ-INPUT.

       3000-TERMINATE.
           PERFORM 9000-CLOSE-FILES
           DISPLAY 'ACCTLOAD - ACCOUNT MASTER LOAD COMPLETE'
           MOVE WS-READ-COUNT TO WS-EDIT-COUNT
           DISPLAY '  RECORDS READ ............ ' WS-EDIT-COUNT
           MOVE WS-CHECKING-COUNT TO WS-EDIT-COUNT
           DISPLAY '  CHECKING ACCOUNTS ....... ' WS-EDIT-COUNT
           MOVE WS-SAVINGS-COUNT TO WS-EDIT-COUNT
           DISPLAY '  SAVINGS ACCOUNTS ........ ' WS-EDIT-COUNT
           MOVE WS-TOTAL-BALANCE TO WS-EDIT-AMOUNT
           DISPLAY '  TOTAL LEDGER BALANCE .... ' WS-EDIT-AMOUNT
           MOVE ZERO TO RETURN-CODE.

       8000-READ-INPUT.
           READ ACCT-IN
               AT END
                   SET END-OF-INPUT TO TRUE
               NOT AT END
                   ADD 1 TO WS-READ-COUNT
           END-READ.

       9000-CLOSE-FILES.
           CLOSE ACCT-IN
                 ACCT-OUT.
