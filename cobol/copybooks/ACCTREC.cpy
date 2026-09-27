      *================================================================*
      * ACCTREC  - ACCOUNT MASTER RECORD            RECFM=FB LRECL=100 *
      * FIRST FRONTIER BANK CORE DEPOSIT SYSTEM. SEQUENCED BY ACCT-ID. *
      * GDG: FFB.ACCTMAST                                              *
      * USE: COPY ACCTREC REPLACING ==:AR:== BY ==XX==.                *
      *================================================================*
       01  :AR:-ACCT-REC.
           05  :AR:-ACCT-ID            PIC X(10).
           05  :AR:-CUST-NAME          PIC X(30).
           05  :AR:-ACCT-TYPE          PIC X(01).
               88  :AR:-CHECKING               VALUE 'C'.
               88  :AR:-SAVINGS                VALUE 'S'.
           05  :AR:-STATUS             PIC X(01).
               88  :AR:-ACTIVE                 VALUE 'A'.
               88  :AR:-FROZEN                 VALUE 'F'.
               88  :AR:-CLOSED                 VALUE 'C'.
           05  :AR:-OPEN-DATE          PIC 9(08).
           05  :AR:-BALANCE            PIC S9(11)V99   COMP-3.
           05  :AR:-INT-RATE           PIC 9V9(04)     COMP-3.
           05  :AR:-OD-LIMIT           PIC S9(07)V99   COMP-3.
           05  :AR:-ACCR-INT           PIC S9(07)V9(06) COMP-3.
           05  :AR:-LAST-ACTIVITY      PIC 9(08).
           05  :AR:-TXN-COUNT-MTD      PIC S9(05)      COMP-3.
           05  FILLER                  PIC X(17).
