       IDENTIFICATION DIVISION.
       PROGRAM-ID. ACCTPOST.
       AUTHOR. FIRST FRONTIER BANK - CORE DEPOSITS.
      *================================================================*
      * ACCTPOST - POST VALIDATED TRANSACTIONS TO THE ACCOUNT MASTER   *
      *                                                                *
      * CLASSIC SEQUENTIAL MASTER UPDATE (BALANCED LINE): THE OLD      *
      * MASTER AND THE TRANSACTIONS ARE BOTH IN ACCOUNT SEQUENCE AND   *
      * ARE MATCHED ON ACCOUNT ID TO PRODUCE A NEW MASTER GENERATION.  *
      * EVERY TRANSACTION IS JOURNALED, POSTED OR REJECTED.            *
      *                                                                *
      *   ACCTOLD  ACCOUNT MASTER (0)   (ACCTREC, FB 100)       INPUT  *
      *   TRANIN   VALID TRANSACTIONS   (TRANREC, FB 80)        INPUT  *
      *   ACCTNEW  UPDATED MASTER       (ACCTREC, FB 100)       OUTPUT *
      *   POSTLOG  POSTING JOURNAL      (POSTREC, FB 100)       OUTPUT *
      *                                                                *
      * POSTING RULES                                                  *
      *   CLSD  CLOSED ACCOUNT - NOTHING POSTS                         *
      *   FRZN  FROZEN ACCOUNT - CREDITS POST, DEBITS ARE REFUSED      *
      *   NSF   DEBIT WOULD TAKE THE BALANCE BELOW THE OVERDRAFT LIMIT *
      *         (BANK FEES ALWAYS POST)                                *
      *   NOAC  NO SUCH ACCOUNT ON THE MASTER                          *
      *                                                                *
      *   RC  0  POSTED (BUSINESS REJECTS ARE NORMAL, NOT AN ERROR)    *
      *   RC 12  INPUT OUT OF SEQUENCE - NEW MASTER IS NOT USABLE      *
      *   RC 16  FILE ERROR                                            *
      *================================================================*
       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT ACCT-OLD    ASSIGN TO ACCTOLD
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-OLD-STATUS.
           SELECT TRAN-IN     ASSIGN TO TRANIN
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-TRAN-STATUS.
           SELECT ACCT-NEW    ASSIGN TO ACCTNEW
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-NEW-STATUS.
           SELECT POST-LOG    ASSIGN TO POSTLOG
                              ORGANIZATION IS SEQUENTIAL
                              FILE STATUS IS WS-LOG-STATUS.

       DATA DIVISION.
       FILE SECTION.
       FD  ACCT-OLD
           RECORDING MODE IS F.
       01  OLD-MASTER-REC              PIC X(100).

       FD  TRAN-IN
           RECORDING MODE IS F.
       COPY TRANREC REPLACING ==:TR:== BY ==TR==.

       FD  ACCT-NEW
           RECORDING MODE IS F.
       01  NEW-MASTER-REC              PIC X(100).

       FD  POST-LOG
           RECORDING MODE IS F.
       COPY POSTREC REPLACING ==:PR:== BY ==PR==.

       WORKING-STORAGE SECTION.
      *    THE MASTER RECORD CURRENTLY BEING UPDATED
       COPY ACCTREC REPLACING ==:AR:== BY ==WM==.

       01  WS-FILE-STATUSES.
           05  WS-OLD-STATUS           PIC X(02).
           05  WS-TRAN-STATUS          PIC X(02).
           05  WS-NEW-STATUS           PIC X(02).
           05  WS-LOG-STATUS           PIC X(02).

       01  WS-MASTER-KEY               PIC X(10) VALUE LOW-VALUES.
       01  WS-TRAN-KEY                 PIC X(10) VALUE LOW-VALUES.
       01  WS-REASON                   PIC X(04).
       01  WS-BAL-AFTER                PIC S9(11)V99 COMP-3.
       01  WS-PROJECTED                PIC S9(13)V99 COMP-3.

       01  WS-COUNTERS.
           05  WS-MASTERS-READ         PIC 9(09) COMP VALUE ZERO.
           05  WS-MASTERS-WRITTEN      PIC 9(09) COMP VALUE ZERO.
           05  WS-MASTERS-UPDATED      PIC 9(09) COMP VALUE ZERO.
           05  WS-TRANS-READ           PIC 9(09) COMP VALUE ZERO.
           05  WS-TRANS-POSTED         PIC 9(09) COMP VALUE ZERO.
           05  WS-REJ-NOAC             PIC 9(09) COMP VALUE ZERO.
           05  WS-REJ-CLSD             PIC 9(09) COMP VALUE ZERO.
           05  WS-REJ-FRZN             PIC 9(09) COMP VALUE ZERO.
           05  WS-REJ-NSF              PIC 9(09) COMP VALUE ZERO.
       01  WS-TOTALS.
           05  WS-CREDITS-POSTED       PIC S9(13)V99 COMP-3 VALUE ZERO.
           05  WS-DEBITS-POSTED        PIC S9(13)V99 COMP-3 VALUE ZERO.
           05  WS-NET-POSTED           PIC S9(13)V99 COMP-3 VALUE ZERO.
       01  WS-UPDATED-SW               PIC X(01) VALUE 'N'.
           88  MASTER-UPDATED                    VALUE 'Y'.

       01  WS-EDIT-COUNT               PIC ZZZ,ZZZ,ZZ9.
       01  WS-EDIT-AMOUNT              PIC -Z,ZZZ,ZZZ,ZZZ,ZZ9.99.

       PROCEDURE DIVISION.
       0000-MAINLINE.
           PERFORM 1000-INITIALIZE
           PERFORM 2000-MATCH
               UNTIL WS-MASTER-KEY = HIGH-VALUES
                 AND WS-TRAN-KEY = HIGH-VALUES
           PERFORM 3000-TERMINATE
           STOP RUN.

       1000-INITIALIZE.
           OPEN INPUT  ACCT-OLD TRAN-IN
                OUTPUT ACCT-NEW POST-LOG
           IF WS-OLD-STATUS NOT = '00' OR WS-TRAN-STATUS NOT = '00'
              OR WS-NEW-STATUS NOT = '00' OR WS-LOG-STATUS NOT = '00'
               DISPLAY 'ACCTPOST: OPEN FAILED ' WS-FILE-STATUSES
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           PERFORM 8100-READ-MASTER
           PERFORM 8200-READ-TRAN.

       2000-MATCH.
           EVALUATE TRUE
               WHEN WS-TRAN-KEY < WS-MASTER-KEY
                   PERFORM 2300-NO-SUCH-ACCOUNT
                   PERFORM 8200-READ-TRAN
               WHEN WS-TRAN-KEY = WS-MASTER-KEY
                   PERFORM 2100-APPLY-TRANSACTION
                   PERFORM 8200-READ-TRAN
               WHEN OTHER
                   PERFORM 2400-WRITE-MASTER
                   PERFORM 8100-READ-MASTER
           END-EVALUATE.

       2100-APPLY-TRANSACTION.
           MOVE SPACES TO WS-REASON
           EVALUATE TRUE
               WHEN WM-CLOSED
                   MOVE 'CLSD' TO WS-REASON
               WHEN WM-FROZEN AND TR-AMOUNT < ZERO
                   MOVE 'FRZN' TO WS-REASON
               WHEN TR-AMOUNT < ZERO AND NOT TR-FEE
                   COMPUTE WS-PROJECTED = WM-BALANCE + TR-AMOUNT
                                        + WM-OD-LIMIT
                   IF WS-PROJECTED < ZERO
                       MOVE 'NSF ' TO WS-REASON
                   END-IF
           END-EVALUATE

           IF WS-REASON = SPACES
               ADD TR-AMOUNT TO WM-BALANCE
               ADD 1 TO WM-TXN-COUNT-MTD
               MOVE TR-TS-DATE TO WM-LAST-ACTIVITY
               ADD 1 TO WS-TRANS-POSTED
               ADD TR-AMOUNT TO WS-NET-POSTED
               IF TR-AMOUNT > ZERO
                   ADD TR-AMOUNT TO WS-CREDITS-POSTED
               ELSE
                   SUBTRACT TR-AMOUNT FROM WS-DEBITS-POSTED
               END-IF
               SET MASTER-UPDATED TO TRUE
           ELSE
               EVALUATE WS-REASON
                   WHEN 'CLSD'  ADD 1 TO WS-REJ-CLSD
                   WHEN 'FRZN'  ADD 1 TO WS-REJ-FRZN
                   WHEN 'NSF '  ADD 1 TO WS-REJ-NSF
               END-EVALUATE
           END-IF
           MOVE WM-BALANCE TO WS-BAL-AFTER
           PERFORM 2500-WRITE-JOURNAL.

       2300-NO-SUCH-ACCOUNT.
           MOVE 'NOAC' TO WS-REASON
           ADD 1 TO WS-REJ-NOAC
           MOVE ZERO TO WS-BAL-AFTER
           PERFORM 2500-WRITE-JOURNAL.

       2400-WRITE-MASTER.
           WRITE NEW-MASTER-REC FROM WM-ACCT-REC
           IF WS-NEW-STATUS NOT = '00'
               DISPLAY 'ACCTPOST: WRITE ACCTNEW FAILED, STATUS '
                       WS-NEW-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           ADD 1 TO WS-MASTERS-WRITTEN
           IF MASTER-UPDATED
               ADD 1 TO WS-MASTERS-UPDATED
           END-IF.

       2500-WRITE-JOURNAL.
           MOVE SPACES          TO PR-POST-REC
           MOVE TR-ACCT-ID      TO PR-ACCT-ID
           MOVE TR-TIMESTAMP    TO PR-TIMESTAMP
           MOVE TR-TXN-ID       TO PR-TXN-ID
           MOVE TR-TXN-TYPE     TO PR-TXN-TYPE
           MOVE TR-AMOUNT       TO PR-AMOUNT
           MOVE WS-BAL-AFTER    TO PR-BAL-AFTER
           MOVE TR-DESCRIPTION  TO PR-DESCRIPTION
           IF WS-REASON = SPACES
               SET PR-POSTED TO TRUE
           ELSE
               SET PR-REJECTED TO TRUE
               MOVE WS-REASON TO PR-REASON
           END-IF
           WRITE PR-POST-REC
           IF WS-LOG-STATUS NOT = '00'
               DISPLAY 'ACCTPOST: WRITE POSTLOG FAILED, STATUS '
                       WS-LOG-STATUS
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF.

       3000-TERMINATE.
           CLOSE ACCT-OLD TRAN-IN ACCT-NEW POST-LOG
           DISPLAY 'ACCTPOST - ACCOUNT POSTING'
           MOVE WS-MASTERS-READ TO WS-EDIT-COUNT
           DISPLAY '  MASTERS READ ............ ' WS-EDIT-COUNT
           MOVE WS-MASTERS-WRITTEN TO WS-EDIT-COUNT
           DISPLAY '  MASTERS WRITTEN ......... ' WS-EDIT-COUNT
           MOVE WS-MASTERS-UPDATED TO WS-EDIT-COUNT
           DISPLAY '  MASTERS UPDATED ......... ' WS-EDIT-COUNT
           MOVE WS-TRANS-READ TO WS-EDIT-COUNT
           DISPLAY '  TRANSACTIONS READ ....... ' WS-EDIT-COUNT
           MOVE WS-TRANS-POSTED TO WS-EDIT-COUNT
           DISPLAY '  TRANSACTIONS POSTED ..... ' WS-EDIT-COUNT
           MOVE WS-REJ-NSF TO WS-EDIT-COUNT
           DISPLAY '  REJECTED - NSF .......... ' WS-EDIT-COUNT
           MOVE WS-REJ-FRZN TO WS-EDIT-COUNT
           DISPLAY '  REJECTED - FROZEN ....... ' WS-EDIT-COUNT
           MOVE WS-REJ-CLSD TO WS-EDIT-COUNT
           DISPLAY '  REJECTED - CLOSED ....... ' WS-EDIT-COUNT
           MOVE WS-REJ-NOAC TO WS-EDIT-COUNT
           DISPLAY '  REJECTED - NO ACCOUNT ... ' WS-EDIT-COUNT
           MOVE WS-CREDITS-POSTED TO WS-EDIT-AMOUNT
           DISPLAY '  CREDITS POSTED .......... ' WS-EDIT-AMOUNT
           MOVE WS-DEBITS-POSTED TO WS-EDIT-AMOUNT
           DISPLAY '  DEBITS POSTED ........... ' WS-EDIT-AMOUNT
           MOVE WS-NET-POSTED TO WS-EDIT-AMOUNT
           DISPLAY '  NET CHANGE .............. ' WS-EDIT-AMOUNT
           IF WS-MASTERS-READ NOT = WS-MASTERS-WRITTEN
               DISPLAY 'ACCTPOST: MASTER COUNT MISMATCH'
               MOVE 12 TO RETURN-CODE
           ELSE
               MOVE ZERO TO RETURN-CODE
           END-IF.

       8100-READ-MASTER.
           MOVE 'N' TO WS-UPDATED-SW
           READ ACCT-OLD INTO WM-ACCT-REC
               AT END
                   MOVE HIGH-VALUES TO WS-MASTER-KEY
               NOT AT END
                   ADD 1 TO WS-MASTERS-READ
                   IF WM-ACCT-ID NOT > WS-MASTER-KEY
                       DISPLAY 'ACCTPOST: MASTER OUT OF SEQUENCE AT '
                               WM-ACCT-ID
                       MOVE 12 TO RETURN-CODE
                       STOP RUN
                   END-IF
                   MOVE WM-ACCT-ID TO WS-MASTER-KEY
           END-READ.

       8200-READ-TRAN.
           READ TRAN-IN
               AT END
                   MOVE HIGH-VALUES TO WS-TRAN-KEY
               NOT AT END
                   ADD 1 TO WS-TRANS-READ
                   IF TR-ACCT-ID < WS-TRAN-KEY
                       DISPLAY 'ACCTPOST: TRANSACTIONS OUT OF SEQUENCE'
                               ' AT ' TR-ACCT-ID
                       MOVE 12 TO RETURN-CODE
                       STOP RUN
                   END-IF
                   MOVE TR-ACCT-ID TO WS-TRAN-KEY
           END-READ.
