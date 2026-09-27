      *================================================================*
      * TRANFEED - DAILY TRANSACTION FEED RECORD    RECFM=FB LRECL=80  *
      * ARRIVES UNSORTED FROM CHANNEL SYSTEMS. DISPLAY FORMAT.         *
      * DSN: FFB.DAILY.TRANFEED                                        *
      * USE: COPY TRANFEED REPLACING ==:TF:== BY ==XX==.               *
      *================================================================*
       01  :TF:-FEED-REC.
           05  :TF:-ACCT-ID            PIC X(10).
           05  :TF:-TIMESTAMP.
               10  :TF:-TS-DATE        PIC X(08).
               10  :TF:-TS-TIME        PIC X(06).
           05  :TF:-TXN-ID             PIC X(12).
           05  :TF:-TXN-TYPE           PIC X(02).
               88  :TF:-VALID-TYPE     VALUE 'DP' 'WD' 'FE' 'TI' 'TO'.
               88  :TF:-CREDIT-TYPE    VALUE 'DP' 'TI'.
           05  :TF:-AMOUNT             PIC 9(09)V99.
           05  :TF:-CHANNEL            PIC X(03).
           05  :TF:-DESCRIPTION        PIC X(20).
           05  FILLER                  PIC X(08).
