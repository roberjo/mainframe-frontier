      *================================================================*
      * POSTREC  - POSTING JOURNAL RECORD           RECFM=FB LRECL=100 *
      * ONE PER TRANSACTION PRESENTED TO POSTING, POSTED OR REJECTED.  *
      * GDG: FFB.DAILY.POSTLOG                                         *
      * USE: COPY POSTREC REPLACING ==:PR:== BY ==XX==.                *
      *================================================================*
       01  :PR:-POST-REC.
           05  :PR:-ACCT-ID            PIC X(10).
           05  :PR:-TIMESTAMP.
               10  :PR:-TS-DATE        PIC X(08).
               10  :PR:-TS-TIME        PIC X(06).
           05  :PR:-TXN-ID             PIC X(12).
           05  :PR:-TXN-TYPE           PIC X(02).
           05  :PR:-AMOUNT             PIC S9(09)V99   COMP-3.
           05  :PR:-BAL-AFTER          PIC S9(11)V99   COMP-3.
           05  :PR:-STATUS             PIC X(01).
               88  :PR:-POSTED                 VALUE 'P'.
               88  :PR:-REJECTED               VALUE 'R'.
           05  :PR:-REASON             PIC X(04).
           05  :PR:-DESCRIPTION        PIC X(20).
           05  FILLER                  PIC X(24).
